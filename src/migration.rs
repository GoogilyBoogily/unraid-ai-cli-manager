use crate::{
    configuration::{self, PolicyChange},
    domain::{Context, ManagerError, Result, Tool, ToolSettings},
    environment::{Environment, private_directory, replace_json, write_json},
    generations::StorageLease,
    tools,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fs, io, path::PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Prepared,
    Disabled,
    Committed,
    RolledBack,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleCopy {
    runtime: bool,
    entries: BTreeMap<String, Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format_version: u32,
    tool: Tool,
    phase: Phase,
    copies: Vec<ScheduleCopy>,
    cron_lines: Vec<(usize, String)>,
    legacy_entries: Vec<String>,
    policy_file: String,
    settings_before: ToolSettings,
}

pub(crate) struct Migration {
    storage: StorageLease,
    journal: Journal,
}
fn journal_path(environment: &Environment, tool: Tool) -> PathBuf {
    environment.path(format!(
        "/boot/config/plugins/ai-cli-manager/migration-{}.json",
        tool.name()
    ))
}
fn schedule_path(environment: &Environment, runtime: bool) -> PathBuf {
    environment.path(if runtime {
        "/tmp/user.scripts/schedule.json"
    } else {
        "/boot/config/plugins/user.scripts/schedule.json"
    })
}
fn wrapper_paths(tool: Tool) -> Vec<String> {
    ["restore", "backup", "backup-on-stop"]
        .iter()
        .map(|action| {
            format!(
                "/boot/config/plugins/user.scripts/scripts/{}-{action}/script",
                tool.legacy_name()
            )
        })
        .collect()
}
fn read_schedule(environment: &Environment, runtime: bool) -> Result<Value> {
    let value: Value = serde_json::from_slice(
        &fs::read(schedule_path(environment, runtime))
            .context("UserScripts schedule copy is missing")?,
    )?;
    if !value.is_object() {
        return Err(ManagerError::Rejected(
            "legacy_mismatch",
            "UserScripts schedule must be an object",
        ));
    }
    Ok(value)
}
fn owned_cron(line: &str, tool: Tool) -> bool {
    let fields: Vec<_> = line.split_whitespace().collect();
    fields.get(5) == Some(&"/usr/local/emhttp/plugins/user.scripts/startCustom.php")
        && fields
            .get(6)
            .is_some_and(|path| wrapper_paths(tool).iter().any(|expected| expected == path))
}
fn cron(environment: &Environment) -> Result<String> {
    match configuration::read_optional(
        &environment.path("/boot/config/plugins/user.scripts/customSchedule.cron"),
    )? {
        Some(bytes) => String::from_utf8(bytes).map_err(|_| {
            ManagerError::Rejected("legacy_mismatch", "UserScripts cron is not UTF-8")
        }),
        None => Ok(String::new()),
    }
}
fn disabled(before: &Value) -> Value {
    let mut value = before.clone();
    value["frequency"] = Value::String("disabled".to_owned());
    value
}
fn legacy_names(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Claude => &[".claude", ".claude.json", "bin"],
        Tool::Codex => &[".codex"],
    }
}
fn exists(path: &std::path::Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ManagerError::Io {
            context: "cannot inspect legacy state",
            source,
        }),
    }
}

fn verify_sources(environment: &Environment, tool: Tool) -> Result<bool> {
    let source = environment.path(format!(
        "/boot/config/{}/{}-persist.sh",
        tool.legacy_name(),
        tool.name()
    ));
    let Some(bytes) = configuration::read_optional(&source)? else {
        return Ok(false);
    };
    let expected: &[u8] = match tool {
        Tool::Claude => include_bytes!("../tests/fixtures/legacy/claude-persist.sh"),
        Tool::Codex => include_bytes!("../tests/fixtures/legacy/codex-persist.sh"),
    };
    if bytes != expected {
        return Err(ManagerError::Rejected(
            "legacy_mismatch",
            "legacy persistence source differs from the reviewed reference",
        ));
    }
    if !environment.path(tool.marker()).is_file() {
        return Err(ManagerError::Rejected(
            "needs_attention",
            "legacy restore marker is missing; restore through the reviewed legacy setup before takeover",
        ));
    }
    for (index, path) in wrapper_paths(tool).iter().enumerate() {
        let body =
            fs::read_to_string(environment.path(path)).context("legacy wrapper is missing")?;
        if let Some(first) = body.lines().find(|line| !line.trim().is_empty())
            && first.starts_with("#!")
            && ![
                "#!/bin/bash",
                "#!/bin/sh",
                "#!/usr/bin/env bash",
                "#!/usr/bin/env sh",
            ]
            .contains(&first.trim())
        {
            return Err(ManagerError::Rejected(
                "legacy_mismatch",
                "legacy wrapper has an unexpected interpreter",
            ));
        }
        let commands: Vec<_> = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect();
        let command = format!(
            "/boot/config/{}/{}-persist.sh {}",
            tool.legacy_name(),
            tool.name(),
            if index == 0 { "restore" } else { "backup" }
        );
        let allowed = [
            command.clone(),
            format!("bash {command}"),
            format!("exec {command}"),
            format!("exec bash {command}"),
            format!("/bin/bash {command}"),
            format!("exec /bin/bash {command}"),
        ];
        if commands.len() != 1 || !allowed.iter().any(|allowed| allowed == commands[0]) {
            return Err(ManagerError::Rejected(
                "legacy_mismatch",
                "legacy wrapper differs from the reviewed command allowlist",
            ));
        }
    }
    Ok(true)
}

impl Migration {
    fn backup(&self) -> PathBuf {
        self.storage.root().join("legacy-migration-backup")
    }
    fn save(&self, environment: &Environment) -> Result<()> {
        write_json(&journal_path(environment, self.journal.tool), &self.journal)
    }
    fn validate(&self) -> Result<()> {
        let journal = &self.journal;
        if journal.format_version != 1
            || journal.copies.len() != 2
            || journal.copies[0].runtime
            || !journal.copies[1].runtime
            || !journal.policy_file.starts_with("policy-")
            || !journal
                .policy_file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || [b'-', b'.'].contains(&byte))
        {
            return Err(ManagerError::Rejected(
                "invalid_journal",
                "migration journal is invalid",
            ));
        }
        let paths = wrapper_paths(journal.tool);
        for copy in &journal.copies {
            if copy.entries.len() != 3 || paths.iter().any(|path| !copy.entries.contains_key(path))
            {
                return Err(ManagerError::Rejected(
                    "invalid_journal",
                    "migration journal contains unowned schedule entries",
                ));
            }
        }
        if journal
            .cron_lines
            .iter()
            .any(|(_, line)| !owned_cron(line, journal.tool))
            || journal
                .legacy_entries
                .iter()
                .any(|entry| !legacy_names(journal.tool).contains(&entry.as_str()))
        {
            return Err(ManagerError::Rejected(
                "invalid_journal",
                "migration journal contains unowned data",
            ));
        }
        Ok(())
    }

    pub(crate) fn disable(&mut self, environment: &Environment) -> Result<()> {
        let _schedules =
            crate::jobs::lock_file(&environment.path("/run/ai-cli-manager/legacy-schedules.lock"))?;
        refuse_legacy_processes(environment, self.journal.tool)?;
        self.validate()?;
        for copy in &self.journal.copies {
            let mut schedule = read_schedule(environment, copy.runtime)?;
            for (path, before) in &copy.entries {
                if &schedule[path] != before {
                    return Err(ManagerError::Rejected(
                        "migration_conflict",
                        "owned schedule changed during takeover",
                    ));
                }
                schedule[path] = disabled(before);
            }
            write_json(&schedule_path(environment, copy.runtime), &schedule)?;
        }
        let contents = cron(environment)?;
        let filtered: String = contents
            .split_inclusive('\n')
            .filter(|line| !owned_cron(line, self.journal.tool))
            .collect();
        crate::environment::write_bytes(
            &environment.path("/boot/config/plugins/user.scripts/customSchedule.cron"),
            filtered.as_bytes(),
        )?;
        self.journal.phase = Phase::Disabled;
        self.save(environment)
    }

    pub(crate) fn commit(&mut self, environment: &Environment) -> Result<()> {
        for name in &self.journal.legacy_entries {
            let source = self.storage.root().join(name);
            let destination = self.backup().join(name);
            if exists(&source)? && exists(&destination)? {
                return Err(ManagerError::Rejected(
                    "migration_conflict",
                    "legacy backup already contains a conflicting entry",
                ));
            }
            if exists(&source)? {
                fs::rename(source, destination).context("cannot retain legacy migration data")?;
            }
        }
        tools::protect_state(&self.backup())?;
        self.journal.phase = Phase::Committed;
        self.save(environment)
    }

    pub(crate) fn rollback(&mut self, environment: &Environment) -> Result<()> {
        let _schedules =
            crate::jobs::lock_file(&environment.path("/run/ai-cli-manager/legacy-schedules.lock"))?;
        self.validate()?;
        let mut failure: Option<ManagerError> = None;
        for copy in &self.journal.copies {
            let result = (|| {
                let mut schedule = read_schedule(environment, copy.runtime)?;
                for (path, before) in &copy.entries {
                    let current = &schedule[path];
                    if current != before && current != &disabled(before) {
                        return Err(ManagerError::Rejected(
                            "migration_conflict",
                            "rollback would overwrite a changed owned schedule",
                        ));
                    }
                    schedule[path] = before.clone();
                }
                write_json(&schedule_path(environment, copy.runtime), &schedule)
            })();
            if let Err(error) = result {
                failure = Some(match failure {
                    Some(previous) => previous.after_rollback(Err(error)),
                    None => error,
                });
            }
        }
        let result = (|| {
            let contents = cron(environment)?;
            let mut lines: Vec<String> =
                contents.split_inclusive('\n').map(str::to_owned).collect();
            if lines.iter().any(|line| {
                owned_cron(line, self.journal.tool)
                    && !self
                        .journal
                        .cron_lines
                        .iter()
                        .any(|(_, before)| before == line)
            }) {
                return Err(ManagerError::Rejected(
                    "migration_conflict",
                    "owned cron entries changed during rollback",
                ));
            }
            for (index, line) in &self.journal.cron_lines {
                if lines.iter().any(|current| current == line) {
                    continue;
                }
                lines.insert((*index).min(lines.len()), line.clone());
            }
            crate::environment::write_bytes(
                &environment.path("/boot/config/plugins/user.scripts/customSchedule.cron"),
                lines.concat().as_bytes(),
            )?;
            for name in &self.journal.legacy_entries {
                let original = self.storage.root().join(name);
                let backup = self.backup().join(name);
                if exists(&original)? && exists(&backup)? {
                    return Err(ManagerError::Rejected(
                        "migration_conflict",
                        "legacy state exists in both rollback locations",
                    ));
                }
                if exists(&backup)? {
                    fs::rename(backup, original)
                        .context("cannot restore original legacy layout")?;
                }
            }
            let policy: Vec<PolicyChange> = serde_json::from_slice(
                &fs::read(self.backup().join(&self.journal.policy_file))
                    .context("migration policy snapshot is missing")?,
            )?;
            configuration::rollback_policy(environment, &policy)?;
            configuration::restore_tool_settings(
                environment,
                self.journal.tool,
                &self.journal.settings_before,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            failure = Some(match failure {
                Some(previous) => previous.after_rollback(Err(error)),
                None => error,
            });
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.journal.phase = Phase::RolledBack;
        self.save(environment)
    }
}

pub(crate) fn begin(
    environment: &Environment,
    tool: Tool,
    policy: &[PolicyChange],
) -> Result<Option<Migration>> {
    if !verify_sources(environment, tool)? {
        return Ok(None);
    }
    refuse_legacy_processes(environment, tool)?;
    if let Some(bytes) = configuration::read_optional(&journal_path(environment, tool))? {
        let journal: Journal = serde_json::from_slice(&bytes)?;
        if journal.tool != tool {
            return Err(ManagerError::Rejected(
                "invalid_journal",
                "migration tool identity does not match",
            ));
        }
        if journal.phase == Phase::Committed {
            return Ok(None);
        }
        if journal.phase != Phase::RolledBack {
            let mut prior = Migration {
                storage: StorageLease::acquire(environment, tool, true)?,
                journal,
            };
            prior.rollback(environment)?;
            crate::unraid::reconcile_cron(environment)?;
            return Err(ManagerError::Rejected(
                "needs_attention",
                "interrupted migration was rolled back; review status and initialize again",
            ));
        }
    }
    let mut copies = Vec::new();
    for runtime in [false, true] {
        let schedule = read_schedule(environment, runtime)?;
        let mut entries = BTreeMap::new();
        for path in wrapper_paths(tool) {
            let entry = &schedule[&path];
            if entry["script"] != path || !entry["frequency"].is_string() {
                return Err(ManagerError::Rejected(
                    "legacy_mismatch",
                    "expected legacy schedule entry is missing or invalid",
                ));
            }
            entries.insert(path, entry.clone());
        }
        copies.push(ScheduleCopy { runtime, entries });
    }
    let contents = cron(environment)?;
    let cron_lines = contents
        .split_inclusive('\n')
        .enumerate()
        .filter(|(_, line)| owned_cron(line, tool))
        .map(|(index, line)| (index, line.to_owned()))
        .collect();
    let storage = StorageLease::acquire(environment, tool, true)?;
    let backup = storage.root().join("legacy-migration-backup");
    if exists(&backup)? && configuration::read_optional(&journal_path(environment, tool))?.is_none()
    {
        return Err(ManagerError::Rejected(
            "migration_conflict",
            "unowned legacy migration backup already exists",
        ));
    }
    private_directory(&backup)?;
    let temporary = tempfile::Builder::new()
        .prefix("policy-")
        .suffix(".json")
        .tempfile_in(&backup)
        .context("cannot preserve migration policy")?;
    let (file, path) = temporary.keep().map_err(|error| ManagerError::Io {
        context: "cannot preserve policy snapshot",
        source: error.error,
    })?;
    drop(file);
    replace_json(&path, &policy)?;
    let mut legacy_entries = Vec::new();
    for name in legacy_names(tool) {
        if exists(&storage.root().join(name))? {
            legacy_entries.push((*name).to_owned());
        }
    }
    let journal = Journal {
        format_version: 1,
        tool,
        phase: Phase::Prepared,
        copies,
        cron_lines,
        legacy_entries,
        policy_file: path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or(ManagerError::Rejected(
                "invalid_journal",
                "invalid policy snapshot name",
            ))?
            .to_owned(),
        settings_before: environment.settings()?.tool(tool).clone(),
    };
    let transaction = Migration { storage, journal };
    transaction.validate()?;
    transaction.save(environment)?;
    Ok(Some(transaction))
}

fn refuse_legacy_processes(environment: &Environment, tool: Tool) -> Result<()> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    let source = environment.path(format!(
        "/boot/config/{}/{}-persist.sh",
        tool.legacy_name(),
        tool.name()
    ));
    for entry in fs::read_dir("/proc").context("cannot inspect legacy jobs")? {
        let entry = entry.context("cannot inspect legacy process")?;
        if entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot inspect legacy process owner",
                    source,
                });
            }
        };
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            continue;
        }
        let command = match fs::read(entry.path().join("cmdline")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot inspect legacy job arguments",
                    source,
                });
            }
        };
        if command
            .split(|byte| *byte == 0)
            .any(|argument| argument == source.as_os_str().as_bytes())
        {
            return Err(ManagerError::Rejected(
                "busy",
                "a legacy persistence job is running or queued; let it finish before takeover",
            ));
        }
    }
    Ok(())
}

pub(crate) fn recovery_required(environment: &Environment, tool: Tool) -> Result<bool> {
    let Some(bytes) = configuration::read_optional(&journal_path(environment, tool))? else {
        return Ok(false);
    };
    let journal: Journal = serde_json::from_slice(&bytes)?;
    if journal.format_version != 1 || journal.tool != tool {
        return Err(ManagerError::Rejected(
            "invalid_journal",
            "migration journal identity is invalid",
        ));
    }
    Ok(![Phase::Committed, Phase::RolledBack].contains(&journal.phase))
}
