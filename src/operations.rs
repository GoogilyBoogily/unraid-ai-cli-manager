use crate::{
    configuration,
    domain::{
        AttemptOutcome, Context, ManagerError, Operation, OperationOutcome, Result, SafeFailure,
        Tool,
    },
    environment::{Environment, private_directory, replace_json},
    generations, process, tools,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    ffi::OsString,
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    pub(crate) version: String,
    pub(crate) server_verified: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    claude: Option<Target>,
    codex: Option<Target>,
}

pub(crate) fn target(environment: &Environment, tool: Tool) -> Result<Option<Target>> {
    let bytes = include_bytes!("../packaging/approved-versions.json").to_vec();
    #[cfg(feature = "test-support")]
    let mut bytes = bytes;
    #[cfg(feature = "test-support")]
    if environment.fixture
        && let Some(fixture) = configuration::read_optional(
            &environment.path("/boot/config/plugins/ai-cli-manager/test-approved-versions.json"),
        )?
    {
        bytes = fixture;
    }
    #[cfg(not(feature = "test-support"))]
    let _ = environment;
    let catalog: Catalog = serde_json::from_slice(&bytes)?;
    let selected = match tool {
        Tool::Claude => catalog.claude,
        Tool::Codex => catalog.codex,
    };
    if let Some(selected) = &selected
        && version_numbers(&selected.version).is_none()
    {
        return Err(ManagerError::Rejected(
            "invalid_catalog",
            "approved target must have a stable numeric version",
        ));
    }
    Ok(selected)
}
fn version_numbers(version: &str) -> Option<Vec<u64>> {
    let numbers: Option<Vec<_>> = version
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    numbers.filter(|numbers| numbers.len() == 3)
}

fn persist_install(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
    version: &str,
) -> OperationOutcome {
    match generations::capture(environment, tool, deadline) {
        Ok(generation) => {
            let receipt = generation
                .path
                .parent()
                .and_then(Path::parent)
                .expect("generation root")
                .join("last-update.json");
            let result = generations::timestamp().and_then(|completed_at| {
                replace_json(
                    &receipt,
                    &json!({"version":version,"completed_at":completed_at}),
                )
            });
            match result {
                Ok(()) => OperationOutcome {
                    installation: AttemptOutcome::Succeeded,
                    persistence: AttemptOutcome::Succeeded,
                    details: json!({"version":version}),
                    failure: None,
                },
                Err(error) => OperationOutcome {
                    installation: AttemptOutcome::Succeeded,
                    persistence: AttemptOutcome::Failed,
                    details: json!({"version":version}),
                    failure: Some(SafeFailure::from(&error)),
                },
            }
        }
        Err(error) => OperationOutcome {
            installation: AttemptOutcome::Succeeded,
            persistence: AttemptOutcome::Failed,
            details: json!({"version":version}),
            failure: Some(SafeFailure::from(&error)),
        },
    }
}

fn install_candidate(
    environment: &Environment,
    tool: Tool,
    target: &Target,
    deadline: Instant,
) -> Result<tools::PackageSelection> {
    let staging_parent = environment.path("/root/.ai-cli-manager/install");
    private_directory(&staging_parent)?;
    let staging = tempfile::tempdir_in(staging_parent).context("cannot stage vendor installer")?;
    let home = staging.path().join("home");
    private_directory(&home)?;
    let installer = staging.path().join("installer.sh");
    let url = match tool {
        Tool::Claude => "https://claude.ai/install.sh",
        Tool::Codex => "https://chatgpt.com/codex/install.sh",
    };
    let download: Vec<OsString> = [
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--tlsv1.2",
        "--connect-timeout",
        "10",
        "--max-time",
        "60",
        "--output",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([installer.as_os_str().to_os_string(), OsString::from(url)])
    .collect();
    process::run_checked(
        environment,
        "curl",
        &download,
        deadline.min(Instant::now() + Duration::from_secs(65)),
    )?;
    let mut arguments = vec![installer.as_os_str().to_os_string()];
    if tool == Tool::Codex {
        arguments.push(OsString::from("--release"));
    }
    arguments.push(OsString::from(&target.version));
    let output = process::run_path(
        environment,
        &environment.program(if tool == Tool::Claude { "bash" } else { "sh" })?,
        &arguments,
        deadline,
        None,
        Some(&home),
    )?;
    if !output.success {
        return Err(ManagerError::Rejected(
            "installer_failed",
            "official vendor installer failed; live selection was retained",
        ));
    }
    let selection = tools::inspect_root(&home, tool)?;
    if selection.version != target.version {
        return Err(ManagerError::Rejected(
            "invalid_package",
            "installer selected an unexpected version",
        ));
    }
    let output = process::run_path(
        environment,
        &home.join(&selection.relative_executable),
        &[OsString::from("--version")],
        deadline.min(Instant::now() + Duration::from_secs(10)),
        None,
        Some(&home),
    )?;
    if !output.success
        || !std::str::from_utf8(&output.stdout)
            .map_err(|_| ManagerError::Rejected("invalid_package", "version output is not UTF-8"))?
            .split_whitespace()
            .any(|part| part == target.version)
    {
        return Err(ManagerError::Rejected(
            "invalid_package",
            "candidate executable did not validate its requested version",
        ));
    }
    let live = environment.path("/root");
    match tool {
        Tool::Claude => tools::transfer(
            environment,
            &home.join(".local/share/claude/versions"),
            &live.join(".local/share/claude/versions"),
            &[],
            deadline,
        )?,
        Tool::Codex => tools::transfer(
            environment,
            &home.join(".codex/packages"),
            &live.join(".codex/packages"),
            &[
                "/standalone/current",
                "/standalone/auto-update-version",
                "/app-server-daemon/",
                "install.lock",
                "install.lock.d",
            ],
            deadline,
        )?,
    }
    tools::validate_payload(&live, &live, tool, &selection)?;
    tools::install_launchers(environment, tool, &selection)?;
    tools::inspect_live(environment, tool)?;
    Ok(selection)
}

pub(crate) fn update(
    environment: &Environment,
    tool: Tool,
    deadline: Instant,
) -> Result<OperationOutcome> {
    if !environment.storage_available()? {
        return Err(ManagerError::Rejected(
            "waiting_for_storage",
            "user shares are not mounted",
        ));
    }
    if !environment.path(tool.marker()).is_file() {
        return Err(ManagerError::Rejected(
            "initialization_required",
            "restore marker is missing; refusing update",
        ));
    }
    let current = tools::inspect_live(environment, tool)?;
    let target = target(environment, tool)?
        .filter(|target| target.server_verified)
        .ok_or(ManagerError::Rejected(
            "unapproved_target",
            "no server-validated vendor target is approved for this manager release",
        ))?;
    let installed = version_numbers(&current.version).ok_or(ManagerError::Rejected(
        "needs_attention",
        "installed version cannot be safely ordered; refusing automatic replacement",
    ))?;
    if installed >= version_numbers(&target.version).expect("target validated") {
        return Ok(OperationOutcome::unchanged(
            json!({"version":current.version,"reason":"same_or_newer_version"}),
        ));
    }
    let _previous = generations::capture(environment, tool, deadline)?;
    let policy = configuration::plan_policy(environment, tool, false)?;
    let result = (|| {
        configuration::apply_policy(environment, &policy)?;
        configuration::record_policy(environment, tool)?;
        install_candidate(environment, tool, &target, deadline)
    })();
    let selected = match result {
        Ok(selected) => selected,
        Err(error) => {
            let rollback = tools::install_launchers(environment, tool, &current)
                .and_then(|()| configuration::rollback_policy(environment, &policy));
            return Err(error.after_rollback(rollback));
        }
    };
    Ok(persist_install(
        environment,
        tool,
        deadline,
        &selected.version,
    ))
}

pub(crate) fn initialize(
    environment: &Environment,
    tool: Tool,
    consent: bool,
    deadline: Instant,
) -> Result<OperationOutcome> {
    if !environment.storage_available()? {
        return Err(ManagerError::Rejected(
            "waiting_for_storage",
            "user shares are not mounted",
        ));
    }
    let launcher = environment.path(tool.launcher());
    let launcher_present = match fs::symlink_metadata(&launcher) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(ManagerError::Io {
                context: "cannot inspect existing installation",
                source,
            });
        }
    };
    if !launcher_present
        && environment
            .path(format!("{}/current", tool.persistent()))
            .exists()
    {
        generations::restore(environment, tool, deadline)?;
    }
    let fresh = !launcher_present && !environment.path(tool.launcher()).exists();
    if !fresh
        && !environment.path(tool.marker()).is_file()
        && tools::active_tool(environment, tool)?
    {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "active sessions have no valid initialization marker",
        ));
    }
    if !fresh {
        tools::inspect_live(environment, tool)?;
    }
    let policy = configuration::plan_policy(environment, tool, fresh && consent)?;
    let before = environment.settings()?;
    let mut migration = crate::migration::begin(environment, tool, &policy)?;
    let result: Result<OperationOutcome> = (|| {
        configuration::apply_policy(environment, &policy)?;
        if fresh {
            let selected = target(environment, tool)?
                .filter(|target| target.server_verified)
                .ok_or(ManagerError::Rejected(
                    "unapproved_target",
                    "fresh installation needs an approved vendor target",
                ))?;
            install_candidate(environment, tool, &selected, deadline)?;
        }
        let generation = generations::capture_initial(environment, tool, deadline)?;
        if let Some(transaction) = &mut migration {
            transaction.disable(environment)?;
        }
        configuration::activate(environment, tool)?;
        configuration::record_policy(environment, tool)?;
        crate::environment::write_bytes(
            &environment.path(tool.marker()),
            b"ai-cli-manager initialized\n",
        )?;
        if let Some(transaction) = &mut migration {
            transaction.commit(environment)?;
        }
        crate::unraid::reconcile_cron(environment)?;
        Ok(OperationOutcome::persisted(
            json!({"generation_identifier":generation.manifest.generation_identifier}),
        ))
    })();
    if let Err(error) = result {
        let rollback = (|| {
            if let Some(transaction) = &mut migration {
                transaction.rollback(environment)?;
            }
            configuration::restore_tool_settings(environment, tool, before.tool(tool))?;
            configuration::rollback_policy(environment, &policy)?;
            crate::unraid::reconcile_cron(environment)
        })();
        return Err(error.after_rollback(rollback));
    }
    result
}

pub(crate) fn execute(
    environment: &Environment,
    tool: Tool,
    operation: Operation,
    consent: bool,
    deadline: Instant,
) -> Result<OperationOutcome> {
    match operation {
        Operation::Initialize => initialize(environment, tool, consent, deadline),
        Operation::Install | Operation::Update => update(environment, tool, deadline),
        Operation::Backup => {
            let generation = generations::capture(environment, tool, deadline)?;
            Ok(OperationOutcome::persisted(
                json!({"generation_identifier":generation.manifest.generation_identifier}),
            ))
        }
        Operation::Restore => generations::restore(environment, tool, deadline),
        #[cfg(feature = "test-support")]
        Operation::Probe => {
            process::run_checked(environment, "probe", &[], deadline)?;
            Ok(OperationOutcome::unchanged(json!({})))
        }
    }
}

pub(crate) fn status(
    environment: &Environment,
    selection: crate::domain::ToolSelection,
) -> Result<serde_json::Value> {
    use crate::{
        domain::{JobState, SafeFailure},
        jobs,
    };
    let settings = environment.settings()?;
    let storage = environment.storage_available()?;
    let mut reports = serde_json::Map::new();
    for tool in selection.tools() {
        let setting = settings.tool(tool);
        let mut errors = Vec::new();
        if crate::migration::recovery_required(environment, tool)? {
            errors.push(SafeFailure{code:"migration_recovery_required".to_owned(),message:"migration was interrupted; review and run Initialize/Adopt to recover before scheduling".to_owned()});
        }
        let installed = match tools::inspect_live(environment, tool) {
            Ok(package) => Some(package.version),
            Err(error) => {
                if setting.initialized {
                    errors.push(SafeFailure::from(&error));
                }
                None
            }
        };
        let cached = if storage {
            match generations::status_summary(environment, tool) {
                Ok(summary) => summary,
                Err(error) => {
                    errors.push(SafeFailure::from(&error));
                    json!({})
                }
            }
        } else {
            json!({})
        };
        let latest = match jobs::latest(environment, tool) {
            Ok(job) => job,
            Err(error) => {
                errors.push(SafeFailure::from(&error));
                None
            }
        };
        let busy = match jobs::active(environment, tool) {
            Ok(active) => active,
            Err(error) => {
                errors.push(SafeFailure::from(&error));
                false
            }
        };
        if let Some(job) = &latest
            && job.state == JobState::Failed
        {
            errors.push(SafeFailure {
                code: job.error_code.clone().unwrap_or("job_failed".to_owned()),
                message: job
                    .error_message
                    .clone()
                    .unwrap_or("tool job failed".to_owned()),
            });
        }
        let target = target(environment, tool)?;
        let approved = installed.as_ref().is_some_and(|version| {
            target
                .as_ref()
                .is_some_and(|target| target.server_verified && &target.version == version)
        });
        if setting.initialized && !approved {
            errors.push(SafeFailure{code:"unverified_version".to_owned(),message:"installed vendor version has not passed this manager release's server gates; validated persistence remains available".to_owned()});
        }
        let policy_path =
            environment.path(format!("/root/.ai-cli-manager/policy-{}.json", tool.name()));
        let policy: Option<serde_json::Value> = configuration::read_optional(&policy_path)?
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()?;
        let processes = tools::running_processes(environment, tool)?;
        let old_processes: Vec<process::ProcessIdentity> = policy
            .as_ref()
            .and_then(|value| value.get("processes"))
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?
            .unwrap_or_default();
        let policy_current = match configuration::policy_is_current(environment, tool) {
            Ok(current) => current,
            Err(error) => {
                if setting.initialized {
                    errors.push(SafeFailure::from(&error));
                }
                false
            }
        };
        if setting.initialized && !policy_current {
            errors.push(SafeFailure {code:"native_update_policy_changed".to_owned(),message:"native update suppression differs from manager policy; initialize or restore after reviewing configuration".to_owned()});
        }
        let policy_restart = processes
            .iter()
            .any(|entry| old_processes.contains(&entry.0));
        let mut daemon_versions = Vec::new();
        let mut running_previous = false;
        for (_, executable) in &processes {
            if tool == Tool::Codex
                && executable
                    .starts_with(environment.path("/root/.codex/packages/app-server-daemon"))
            {
                let mut found = false;
                for ancestor in executable.ancestors().skip(1) {
                    if !ancestor.starts_with(environment.path("/root/.codex/packages")) {
                        break;
                    }
                    if let Some(bytes) =
                        configuration::read_optional(&ancestor.join("codex-package.json"))?
                    {
                        let metadata: serde_json::Value = serde_json::from_slice(&bytes)?;
                        if let Some(version) = metadata["version"].as_str() {
                            daemon_versions.push(version.to_owned());
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    daemon_versions.push("unknown".to_owned());
                }
            } else if let Ok(selected) = environment.path(tool.launcher()).canonicalize() {
                running_previous |= &selected != executable;
            }
        }
        daemon_versions.sort();
        daemon_versions.dedup();
        let restart_required = policy_restart
            || running_previous
            || daemon_versions
                .iter()
                .any(|version| Some(version) != installed.as_ref());
        let readiness = if !setting.enabled {
            "disabled"
        } else if !storage {
            "waiting_for_storage"
        } else if !environment.path(tool.marker()).is_file() || !errors.is_empty() {
            "needs_attention"
        } else if restart_required {
            "restart_required"
        } else {
            "ready"
        };
        let operation = if busy { "busy" } else { "idle" };
        reports.insert(tool.name().to_owned(),json!({"readiness":readiness,"operation":operation,"settings":setting,"restart_required":restart_required,"versions":{"installed":installed,"cached":cached["version"],"daemon":daemon_versions},"last_successful_backup":cached["last_backup"],"last_successful_update":cached["last_update"],"errors":errors,"job":latest.map(|job|json!({"job_identifier":job.job_identifier,"state":job.state})),"native_update_policy":{"configured":policy_current,"verified_on_server":policy_current && approved,"restart_required":policy_restart}}));
    }
    Ok(
        json!({"schema_version":1,"backup_interval_minutes":settings.backup_interval_minutes,"tools":reports}),
    )
}
