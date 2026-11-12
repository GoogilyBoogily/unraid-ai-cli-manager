use crate::{
    domain::{Context, ManagerError, Result},
    environment::Environment,
};
use rustix::process::{Pid, PidfdFlags, Signal, WaitOptions};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{self, Read},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProcessIdentity {
    pub(crate) identifier: u32,
    pub(crate) start_ticks: u64,
}

fn process_info(identifier: u32) -> Result<Option<(ProcessIdentity, u32)>> {
    let contents = match fs::read_to_string(format!("/proc/{identifier}/stat")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ManagerError::Io {
                context: "cannot inspect owned process",
                source,
            });
        }
    };
    let fields: Vec<_> = contents
        .rsplit_once(')')
        .ok_or(ManagerError::Rejected(
            "process_identity",
            "invalid process identity record",
        ))?
        .1
        .split_whitespace()
        .collect();
    let parent =
        fields
            .get(1)
            .and_then(|value| value.parse().ok())
            .ok_or(ManagerError::Rejected(
                "process_identity",
                "missing process parent",
            ))?;
    let start_ticks =
        fields
            .get(19)
            .and_then(|value| value.parse().ok())
            .ok_or(ManagerError::Rejected(
                "process_identity",
                "missing process start time",
            ))?;
    Ok(Some((
        ProcessIdentity {
            identifier,
            start_ticks,
        },
        parent,
    )))
}

pub(crate) fn identity(identifier: u32) -> Result<ProcessIdentity> {
    process_info(identifier)?
        .map(|entry| entry.0)
        .ok_or(ManagerError::Rejected(
            "process_gone",
            "owned process has exited",
        ))
}

pub(crate) fn alive(expected: &ProcessIdentity) -> Result<bool> {
    Ok(process_info(expected.identifier)?
        .is_some_and(|entry| entry.0.start_ticks == expected.start_ticks))
}

fn descendants() -> Result<Vec<ProcessIdentity>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir("/proc").context("cannot enumerate owned processes")? {
        let entry = entry.context("cannot read process directory")?;
        if let Some(identifier) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        {
            if let Some(info) = process_info(identifier)? {
                entries.push(info);
            }
        }
    }
    let mut owned = HashSet::from([std::process::id()]);
    loop {
        let before = owned.len();
        for (process, parent) in &entries {
            if owned.contains(parent) {
                owned.insert(process.identifier);
            }
        }
        if owned.len() == before {
            break;
        }
    }
    Ok(entries
        .into_iter()
        .filter(|entry| {
            entry.0.identifier != std::process::id() && owned.contains(&entry.0.identifier)
        })
        .map(|entry| entry.0)
        .collect())
}

fn signal(process: &ProcessIdentity, signal: Signal) -> Result<()> {
    if process.identifier <= 1 || !alive(process)? {
        return Ok(());
    }
    let identifier = Pid::from_raw(process.identifier as i32).ok_or(ManagerError::Rejected(
        "process_identity",
        "invalid process identifier",
    ))?;
    let descriptor = match rustix::process::pidfd_open(identifier, PidfdFlags::empty()) {
        Ok(descriptor) => descriptor,
        Err(rustix::io::Errno::SRCH) => return Ok(()),
        Err(error) => {
            return Err(ManagerError::Io {
                context: "cannot open stable process handle",
                source: error.into(),
            });
        }
    };
    if !alive(process)? {
        return Ok(());
    }
    match rustix::process::pidfd_send_signal(descriptor, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(ManagerError::Io {
            context: "cannot signal owned process",
            source: error.into(),
        }),
    }
}

pub(crate) fn enable_subreaper() -> Result<()> {
    rustix::process::set_child_subreaper(Some(Pid::INIT))
        .map_err(io::Error::from)
        .context("cannot supervise detached descendants")
}

fn reap() -> Result<()> {
    loop {
        match rustix::process::wait(WaitOptions::NOHANG) {
            Ok(Some(_)) => continue,
            Ok(None) | Err(rustix::io::Errno::CHILD) => return Ok(()),
            Err(error) => {
                return Err(ManagerError::Io {
                    context: "cannot reap owned descendants",
                    source: error.into(),
                });
            }
        }
    }
}

pub(crate) fn drain_descendants(deadline: Instant) -> Result<()> {
    let escalation = Instant::now()
        + Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now()) / 3);
    loop {
        let processes = descendants()?;
        if processes.is_empty() {
            reap()?;
            return Ok(());
        }
        for process in processes {
            signal(
                &process,
                if Instant::now() >= escalation {
                    Signal::KILL
                } else {
                    Signal::TERM
                },
            )?;
        }
        reap()?;
        if Instant::now() >= deadline {
            let remaining = descendants()?;
            if remaining.is_empty() {
                return Ok(());
            }
            return Err(ManagerError::Rejected(
                "descendants_alive",
                "owned descendants have not exited; shutdown is not drained",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) fn run_path(
    environment: &Environment,
    program: &Path,
    arguments: &[OsString],
    deadline: Instant,
    cancellation: Option<&Path>,
) -> Result<CommandOutput> {
    enable_subreaper()?;
    let mut stdout = tempfile::tempfile().context("cannot create private command output")?;
    let mut stderr = tempfile::tempfile().context("cannot create private command diagnostics")?;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(
            stdout
                .try_clone()
                .context("cannot duplicate command output")?,
        )
        .stderr(
            stderr
                .try_clone()
                .context("cannot duplicate command diagnostics")?,
        )
        .process_group(0);
    command
        .env("HOME", environment.path("/root"))
        .env("CODEX_HOME", environment.path("/root/.codex"))
        .env("DISABLE_AUTOUPDATER", "1")
        .env(
            "PATH",
            "/root/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        );
    let mut child = command.spawn().context("cannot start command")?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let work_deadline = deadline - (remaining / 4).min(Duration::from_secs(2));
    let status = loop {
        if let Some(status) = child.try_wait().context("cannot wait for command")? {
            break status;
        }
        let cancelled = cancellation.is_some_and(Path::exists);
        let too_large = stdout
            .metadata()
            .context("cannot inspect command output")?
            .len()
            > 1_048_576
            || stderr
                .metadata()
                .context("cannot inspect command diagnostics")?
                .len()
                > 1_048_576;
        if cancelled || too_large || Instant::now() >= work_deadline {
            drain_descendants(deadline)?;
            return Err(ManagerError::Rejected(
                if cancelled {
                    "cancelled"
                } else if too_large {
                    "output_limit"
                } else {
                    "deadline_exceeded"
                },
                "command stopped before completion",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    };
    if !descendants()?.is_empty() {
        drain_descendants(deadline)?;
        return Err(ManagerError::Rejected(
            "unexpected_descendants",
            "command left background descendants; they were terminated",
        ));
    }
    use std::io::{Seek, SeekFrom};
    stdout
        .seek(SeekFrom::Start(0))
        .context("cannot read command output")?;
    stderr
        .seek(SeekFrom::Start(0))
        .context("cannot read command diagnostics")?;
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    stdout
        .take(1_048_576)
        .read_to_end(&mut output)
        .context("cannot collect command output")?;
    stderr
        .take(1_048_576)
        .read_to_end(&mut diagnostics)
        .context("cannot collect command diagnostics")?;
    Ok(CommandOutput {
        success: status.success(),
        stdout: output,
        stderr: diagnostics,
    })
}

pub(crate) fn run_checked(
    environment: &Environment,
    name: &str,
    arguments: &[OsString],
    deadline: Instant,
) -> Result<Vec<u8>> {
    let output = run_path(
        environment,
        &environment.program(name)?,
        arguments,
        deadline,
        None,
    )?;
    if !output.success {
        return Err(ManagerError::Rejected(
            "command_failed",
            "required command failed",
        ));
    }
    Ok(output.stdout)
}
