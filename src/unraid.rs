use crate::{
    configuration::read_optional,
    domain::{Context, Result, Tool},
    environment::{Environment, write_bytes},
    process,
};
use std::{
    ffi::OsString,
    time::{Duration, Instant},
};

pub(crate) fn reconcile_cron(environment: &Environment) -> Result<()> {
    let _cron = crate::jobs::lock_file(&environment.path("/run/ai-cli-manager/cron.lock"))?;
    let settings = environment.settings()?;
    let mut cron = String::from("# ai-cli-manager: generated from manager settings\n");
    for tool in Tool::ALL {
        let selected = settings.tool(tool);
        if !selected.initialized
            || !selected.enabled
            || crate::migration::recovery_required(environment, tool)?
        {
            continue;
        }
        let schedule = if settings.backup_interval_minutes == 60 {
            "0 * * * *".to_owned()
        } else {
            format!("*/{} * * * *", settings.backup_interval_minutes)
        };
        cron.push_str(&format!("{schedule} /usr/local/sbin/ai-cli-manager __internal tick --operation backup --tool {} >/dev/null 2>&1\n",tool.name()));
        if selected.automatic_updates {
            cron.push_str(&format!("0 4 * * * /usr/local/sbin/ai-cli-manager __internal tick --operation update --tool {} >/dev/null 2>&1\n",tool.name()));
        }
    }
    let path = environment.path("/boot/config/plugins/ai-cli-manager/schedule.cron");
    if read_optional(&path)?.as_deref() != Some(cron.as_bytes()) {
        write_bytes(&path, cron.as_bytes())?;
    }
    if environment
        .path("/var/log/plugins/ai-cli-manager.plg")
        .exists()
    {
        process::run_checked(
            environment,
            "update_cron",
            &Vec::<OsString>::new(),
            Instant::now() + Duration::from_secs(5),
        )?;
    }
    Ok(())
}

pub(crate) fn notify_job(environment: &Environment, record: &crate::jobs::JobRecord) -> Result<()> {
    use crate::{domain::JobState, environment::write_json};
    use serde_json::json;
    let marker = environment.path(format!(
        "/run/ai-cli-manager/notification-{}.json",
        record.tool.name()
    ));
    if record.state == JobState::Cancelled {
        return Ok(());
    }
    if record.state == JobState::Succeeded {
        return write_json(&marker, &json!({"failure":null}));
    }
    let failure = record.error_code.as_deref().unwrap_or("job_failed");
    if let Some(bytes) = read_optional(&marker)? {
        let previous: serde_json::Value = serde_json::from_slice(&bytes)?;
        if previous["failure"] == failure && previous["delivery_error"].is_null() {
            return Ok(());
        }
    }
    let description = format!(
        "{} operation failed: {}. Open Settings > AI CLI Manager for details.",
        record.tool.name(),
        failure
    );
    let result = process::run_checked(
        environment,
        "notify",
        &[
            "-e".into(),
            "AI CLI Manager".into(),
            "-s".into(),
            "AI CLI Manager needs attention".into(),
            "-d".into(),
            description.into(),
            "-i".into(),
            "warning".into(),
        ],
        Instant::now() + Duration::from_secs(3),
    );
    write_json(
        &marker,
        &json!({"failure":failure,"delivery_error":result.as_ref().err().map(crate::domain::SafeFailure::from)}),
    )?;
    result.map(|_| ())
}

pub(crate) fn event(environment: &Environment, event: &str) -> Result<serde_json::Value> {
    use crate::{domain::ManagerError, jobs};
    use serde_json::json;
    match event {
        "disks_mounted" => {
            if !environment.storage_available()? {
                return Err(ManagerError::Rejected(
                    "waiting_for_storage",
                    "user shares are not mounted",
                ));
            }
            let _admission =
                jobs::lock_file(&environment.path("/run/ai-cli-manager/admission.lock"))?;
            let stopping = environment.path("/run/ai-cli-manager/stopping");
            if stopping.exists() {
                std::fs::remove_file(stopping).context("cannot reset stop admission")?;
            }
            write_bytes(
                &environment.path("/run/ai-cli-manager/storage-available"),
                b"available\n",
            )?;
            use std::os::unix::process::CommandExt;
            std::process::Command::new(std::env::current_exe().context("cannot locate manager")?)
                .args(["__internal", "start"])
                .process_group(0)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("cannot enqueue array initialization")?;
            Ok(json!({"enqueued":true}))
        }
        "stopping" => {
            let _admission =
                jobs::lock_file(&environment.path("/run/ai-cli-manager/admission.lock"))?;
            write_bytes(
                &environment.path("/run/ai-cli-manager/stopping"),
                b"stopping\n",
            )?;
            Ok(json!({"accepting_jobs":false}))
        }
        _ => Err(ManagerError::Rejected(
            "invalid_event",
            "unsupported Unraid event",
        )),
    }
}

pub(crate) fn start(environment: &Environment) -> Result<serde_json::Value> {
    use crate::{domain::Operation, jobs};
    use serde_json::json;
    reconcile_cron(environment)?;
    let settings = environment.settings()?;
    let mut reports = Vec::new();
    for tool in Tool::ALL {
        if settings.tool(tool).enabled && settings.tool(tool).initialized {
            reports.push(
                match jobs::scheduled(environment, tool, Operation::Restore) {
                    Ok(receipt) => json!({"tool":tool,"receipt":receipt}),
                    Err(error) => {
                        json!({"tool":tool,"error":crate::domain::SafeFailure::from(&error)})
                    }
                },
            );
        }
    }
    crate::environment::write_json(
        &environment.path("/run/ai-cli-manager/start.json"),
        &reports,
    )?;
    Ok(json!({"jobs":reports}))
}

fn wait_idle(environment: &Environment, deadline: Instant) -> Result<bool> {
    loop {
        let mut active = false;
        for tool in Tool::ALL {
            active |= crate::jobs::active(environment, tool)?;
        }
        if !active {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) fn shutdown(
    environment: &Environment,
    milliseconds: u64,
    uninstall: bool,
) -> Result<serde_json::Value> {
    use crate::{
        domain::{JobState, ManagerError},
        jobs,
    };
    use serde_json::json;
    if !(100..=120_000).contains(&milliseconds) {
        return Err(ManagerError::Rejected(
            "invalid_deadline",
            "shutdown budget must be 100 to 120000 milliseconds",
        ));
    }
    let started = Instant::now();
    let budget = Duration::from_millis(milliseconds);
    let deadline = started + budget;
    let _shutdown = jobs::lock_file(&environment.path("/run/ai-cli-manager/shutdown.lock"))?;
    event(environment, "stopping")?;
    for tool in Tool::ALL {
        jobs::cancel(environment, tool)?;
    }
    if !wait_idle(environment, started + budget / 6)? {
        if !wait_idle(environment, deadline)? {
            return Err(ManagerError::Rejected(
                "descendants_alive",
                "manager workers have not drained within the shutdown budget",
            ));
        }
        return Err(ManagerError::Rejected(
            "deadline_exceeded",
            "worker cancellation consumed the final backup window",
        ));
    }
    let settings = environment.settings()?;
    let mut reports = Vec::new();
    let backup_deadline = started + budget * 5 / 6;
    if environment.storage_available()? {
        for tool in Tool::ALL {
            if settings.tool(tool).enabled && settings.tool(tool).initialized {
                let remaining = backup_deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis() as u64;
                if remaining < 50 {
                    break;
                }
                reports.push((tool, jobs::final_backup(environment, tool, remaining)));
            }
        }
    }
    if !wait_idle(environment, backup_deadline)? {
        for tool in Tool::ALL {
            jobs::cancel(environment, tool)?;
        }
        if !wait_idle(environment, deadline)? {
            return Err(ManagerError::Rejected(
                "descendants_alive",
                "final backup workers have not drained within the shutdown budget",
            ));
        }
    }
    let mut errors = Vec::new();
    for (tool, receipt) in reports {
        match receipt {
            Err(error) => {
                errors.push(json!({"tool":tool,"error":crate::domain::SafeFailure::from(&error)}))
            }
            Ok(_) => {
                if let Some(record) = jobs::latest(environment, tool)?
                    && record.state != JobState::Succeeded
                {
                    errors.push(
                        json!({"tool":tool,"code":record.error_code,"error":record.error_message}),
                    );
                }
            }
        }
    }
    process::finish_children(deadline)?;
    if uninstall {
        // Empty our own schedule; keep settings and all user/legacy data.
        write_bytes(
            &environment.path("/boot/config/plugins/ai-cli-manager/schedule.cron"),
            b"# ai-cli-manager uninstalled\n",
        )?;
        process::run_checked(environment, "update_cron", &[], deadline)?;
    }
    if !errors.is_empty() {
        crate::environment::write_json(
            &environment.path("/run/ai-cli-manager/shutdown-errors.json"),
            &errors,
        )?;
        return Err(ManagerError::Rejected(
            "final_backup_failed",
            "workers drained but one or more final backups failed; inspect shutdown status",
        ));
    }
    Ok(json!({"drained":true,"uninstalled":uninstall}))
}

pub(crate) fn plugin_hook(
    environment: &Environment,
    kind: &str,
    method: &str,
    plugin: &str,
    error: &str,
) -> Result<serde_json::Value> {
    if kind == "plugin"
        && ["install", "update"].contains(&method)
        && plugin == "ai-cli-manager.plg"
        && error.is_empty()
    {
        reconcile_cron(environment)?;
    }
    Ok(
        serde_json::json!({"reconciled":kind=="plugin" && ["install","update"].contains(&method) && plugin=="ai-cli-manager.plg" && error.is_empty()}),
    )
}
