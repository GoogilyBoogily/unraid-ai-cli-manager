use crate::{
    domain::{
        AttemptOutcome, Context, JobState, ManagerError, Operation, OperationOutcome, Result,
        SafeFailure, Tool,
    },
    environment::{Environment, private_directory, write_json},
    process::{self, ProcessIdentity},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct JobRecord {
    pub(crate) job_identifier: String,
    pub(crate) tool: Tool,
    pub(crate) operation: Operation,
    pub(crate) state: JobState,
    pub(crate) supervisor: Option<ProcessIdentity>,
    pub(crate) timeout_milliseconds: u64,
    pub(crate) host_execution_opt_in: bool,
    pub(crate) installation: AttemptOutcome,
    pub(crate) persistence: AttemptOutcome,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) result: Option<OperationOutcome>,
}

pub(crate) fn lock_file(path: &std::path::Path) -> Result<File> {
    fs::create_dir_all(path.parent().ok_or(ManagerError::Rejected(
        "unsafe_path",
        "lock has no directory",
    ))?)
    .context("cannot create lock parent")?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .context("cannot open manager lock")?;
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(file),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(ManagerError::Rejected(
            "busy",
            "another operation holds the tool lock",
        )),
        Err(error) => Err(ManagerError::Io {
            context: "cannot acquire manager lock",
            source: error.into(),
        }),
    }
}

fn directory(environment: &Environment) -> PathBuf {
    environment.path("/run/ai-cli-manager/jobs")
}
pub(crate) fn record_path(environment: &Environment, identifier: &str) -> Result<PathBuf> {
    if identifier.is_empty()
        || identifier.len() > 128
        || !identifier
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'-')
    {
        return Err(ManagerError::Rejected(
            "invalid_job",
            "invalid job identifier",
        ));
    }
    Ok(directory(environment).join(format!("{identifier}.json")))
}
pub(crate) fn read_job(environment: &Environment, identifier: &str) -> Result<JobRecord> {
    Ok(serde_json::from_slice(
        &fs::read(record_path(environment, identifier)?).context("cannot read job record")?,
    )?)
}

pub(crate) fn active(environment: &Environment, tool: Tool) -> Result<bool> {
    let Some(record) = latest(environment, tool)? else {
        return Ok(false);
    };
    if ![JobState::Queued, JobState::Running, JobState::Cancelling].contains(&record.state) {
        return Ok(false);
    }
    if let Some(supervisor) = &record.supervisor
        && !process::alive(supervisor)?
    {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "previous job supervisor was lost; verify its workers before starting more work",
        ));
    }
    Ok(true)
}

pub(crate) fn latest(environment: &Environment, tool: Tool) -> Result<Option<JobRecord>> {
    let path = directory(environment).join(format!("{}.active", tool.name()));
    let identifier = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ManagerError::Io {
                context: "cannot read active job selector",
                source,
            });
        }
    };
    let record = read_job(environment, &identifier)?;
    if record.tool != tool {
        return Err(ManagerError::Rejected(
            "invalid_job",
            "job tool selector does not match",
        ));
    }
    Ok(Some(record))
}

pub(crate) fn submit(
    environment: &Environment,
    tool: Tool,
    operation: Operation,
    consent: bool,
    timeout_milliseconds: u64,
) -> Result<Value> {
    submit_inner(
        environment,
        tool,
        operation,
        consent,
        timeout_milliseconds,
        false,
        false,
    )
}

pub(crate) fn scheduled(
    environment: &Environment,
    tool: Tool,
    operation: Operation,
) -> Result<Value> {
    submit_inner(environment, tool, operation, false, 600_000, false, true)
}

pub(crate) fn final_backup(
    environment: &Environment,
    tool: Tool,
    timeout_milliseconds: u64,
) -> Result<Value> {
    submit_inner(
        environment,
        tool,
        Operation::Backup,
        false,
        timeout_milliseconds,
        true,
        false,
    )
}

fn submit_inner(
    environment: &Environment,
    tool: Tool,
    operation: Operation,
    consent: bool,
    timeout_milliseconds: u64,
    final_backup: bool,
    scheduled: bool,
) -> Result<Value> {
    let _admission = lock_file(&environment.path("/run/ai-cli-manager/admission.lock"))?;
    if !final_backup && environment.path("/run/ai-cli-manager/stopping").exists() {
        return Err(ManagerError::Rejected(
            "stopping",
            "manager is rejecting work while the array stops",
        ));
    }
    if active(environment, tool)? {
        return Err(ManagerError::Rejected(
            "busy",
            "another job is active for this tool",
        ));
    }
    let _tool_lock = match lock_file(&environment.path(tool.lock())) {
        Ok(lock) => Some(lock),
        Err(ManagerError::Rejected("busy", _)) if operation == Operation::Initialize => None,
        Err(error) => return Err(error),
    };
    let settings = environment.settings()?;
    if operation != Operation::Initialize && crate::migration::recovery_required(environment, tool)?
    {
        return Err(ManagerError::Rejected(
            "migration_recovery_required",
            "recover the interrupted migration through Initialize/Adopt before starting other jobs",
        ));
    }
    if scheduled
        && (!settings.tool(tool).enabled
            || !settings.tool(tool).initialized
            || (operation == Operation::Update && !settings.tool(tool).automatic_updates))
    {
        return Ok(json!({"skipped":"disabled"}));
    }
    if !settings.tool(tool).initialized && operation != Operation::Initialize {
        #[cfg(feature = "test-support")]
        if operation != Operation::Probe {
            return Err(ManagerError::Rejected(
                "initialization_required",
                "initialize the selected tool first",
            ));
        }
        #[cfg(not(feature = "test-support"))]
        return Err(ManagerError::Rejected(
            "initialization_required",
            "initialize the selected tool first",
        ));
    }
    private_directory(&directory(environment))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ManagerError::Rejected("clock_failure", "system clock precedes epoch"))?
        .as_nanos();
    let identifier = format!("{}-{timestamp}-{}", tool.name(), std::process::id());
    let record = JobRecord {
        job_identifier: identifier.clone(),
        tool,
        operation,
        state: JobState::Queued,
        supervisor: None,
        timeout_milliseconds,
        host_execution_opt_in: consent,
        installation: AttemptOutcome::NotAttempted,
        persistence: AttemptOutcome::NotAttempted,
        error_code: None,
        error_message: None,
        result: None,
    };
    write_json(&record_path(environment, &identifier)?, &record)?;
    fs::write(
        directory(environment).join(format!("{}.active", tool.name())),
        &identifier,
    )
    .context("cannot reserve tool job")?;
    let mut child =
        Command::new(std::env::current_exe().context("cannot locate manager executable")?)
            .args(["__internal", "supervise", "--identifier", &identifier])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("cannot start job supervisor")?;
    drop(_tool_lock);
    let acknowledgement_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let record = read_job(environment, &identifier)?;
        if record.supervisor.is_some() || record.state == JobState::Failed {
            return Ok(json!({"job_identifier": identifier}));
        }
        if child
            .try_wait()
            .context("cannot inspect supervisor startup")?
            .is_some()
            || Instant::now() >= acknowledgement_deadline
        {
            return Err(ManagerError::Rejected(
                "supervisor_failed",
                "job supervisor did not acknowledge startup",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) fn supervise(environment: &Environment, identifier: &str) -> Result<Value> {
    let mut record = read_job(environment, identifier)?;
    record.supervisor = Some(process::identity(std::process::id())?);
    write_json(&record_path(environment, identifier)?, &record)?;
    let deadline = Instant::now() + Duration::from_millis(record.timeout_milliseconds);
    let startup_deadline = deadline.min(
        Instant::now()
            + Duration::from_secs(if record.operation == Operation::Initialize {
                60
            } else {
                2
            }),
    );
    let cancellation = directory(environment).join(format!("{identifier}.cancel"));
    let tool_lock = loop {
        if cancellation.exists() {
            record.state = JobState::Cancelled;
            record.error_code = Some("cancelled".to_owned());
            record.error_message =
                Some("queued job cancelled before acquiring its lock".to_owned());
            write_json(&record_path(environment, identifier)?, &record)?;
            return Ok(json!({"state":record.state}));
        }
        match lock_file(&environment.path(record.tool.lock())) {
            Ok(lock) => break lock,
            Err(ManagerError::Rejected("busy", _)) if Instant::now() < startup_deadline => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => {
                record.error_code = Some(error.code().to_owned());
                record.error_message = Some(error.to_string());
                record.state = JobState::Failed;
                write_json(&record_path(environment, identifier)?, &record)?;
                return Err(error);
            }
        }
    };
    record.state = JobState::Running;
    write_json(&record_path(environment, identifier)?, &record)?;
    let output = process::run_path(
        environment,
        &std::env::current_exe().context("cannot locate manager worker")?,
        &[
            OsString::from("__internal"),
            OsString::from("worker"),
            OsString::from("--identifier"),
            OsString::from(identifier),
        ],
        deadline,
        Some(&cancellation),
        None,
    );
    if output
        .as_ref()
        .is_err_and(|error| error.code() == "descendants_alive")
    {
        record.state = JobState::Cancelling;
        record.error_code = Some("descendants_alive".to_owned());
        record.error_message = Some("waiting for owned descendants to exit".to_owned());
        write_json(&record_path(environment, identifier)?, &record)?;
        // Keep the legacy lock while a kernel-blocked writer can still touch storage.
        loop {
            match process::drain_descendants(Instant::now() + Duration::from_secs(1)) {
                Ok(()) => break,
                Err(error) if error.code() == "descendants_alive" => continue,
                Err(error) => return Err(error),
            }
        }
    }
    let completion: std::result::Result<OperationOutcome, SafeFailure> = match output {
        Ok(output) if output.success => {
            serde_json::from_slice(&output.stdout).map_err(|error| SafeFailure {
                code: "worker_protocol".to_owned(),
                message: format!(
                    "worker returned invalid result at line {}, column {}",
                    error.line(),
                    error.column()
                ),
            })
        }
        Ok(output) => Err(
            match serde_json::from_slice::<SafeFailure>(&output.stderr) {
                Ok(failure) => failure,
                Err(error) => SafeFailure {
                    code: "worker_protocol".to_owned(),
                    message: format!(
                        "worker exited without valid error response at line {}, column {}",
                        error.line(),
                        error.column()
                    ),
                },
            },
        ),
        Err(error) => Err(SafeFailure::from(&error)),
    };
    match completion {
        Ok(result) => {
            record.installation = result.installation;
            record.persistence = result.persistence;
            record.state = if result.failure.is_some()
                || result.persistence == AttemptOutcome::Failed
                || result.installation == AttemptOutcome::Failed
            {
                JobState::Failed
            } else {
                JobState::Succeeded
            };
            if let Some(failure) = &result.failure {
                record.error_code = Some(failure.code.clone());
                record.error_message = Some(failure.message.clone());
            }
            record.result = Some(result);
        }
        Err(failure) => {
            let cancelled = failure.code == "cancelled";
            record.error_code = Some(failure.code);
            record.error_message = Some(failure.message);
            record.state = if cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
        }
    }
    drop(tool_lock);
    write_json(&record_path(environment, identifier)?, &record)?;
    crate::unraid::notify_job(environment, &record)?;
    Ok(json!({"state": record.state}))
}

pub(crate) fn cancel(environment: &Environment, tool: Tool) -> Result<()> {
    if active(environment, tool)? {
        let record = latest(environment, tool)?.expect("active job");
        crate::environment::write_bytes(
            &directory(environment).join(format!("{}.cancel", record.job_identifier)),
            b"cancel\n",
        )?;
    }
    Ok(())
}

pub(crate) fn worker(environment: &Environment, identifier: &str) -> Result<OperationOutcome> {
    let record = read_job(environment, identifier)?;
    let parent = rustix::process::getppid().map(|parent| parent.as_raw_nonzero().get() as u32);
    if parent.is_none() || record.supervisor.as_ref().map(|parent| parent.identifier) != parent {
        return Err(ManagerError::Rejected(
            "invalid_worker",
            "worker must be started by its registered supervisor",
        ));
    }
    let duration = Duration::from_millis(record.timeout_milliseconds);
    crate::execute_operation(
        environment,
        record.tool,
        record.operation,
        record.host_execution_opt_in,
        Instant::now() + duration - (duration / 4).min(Duration::from_secs(2)),
    )
}
