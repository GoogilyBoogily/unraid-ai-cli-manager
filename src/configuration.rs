use crate::{
    domain::{Context, ManagerError, Result, Settings, Tool},
    environment::{Environment, write_bytes, write_json},
    jobs,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ManagerError::Io {
            context: "cannot read manager-controlled file",
            source,
        }),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PolicyFile {
    ClaudeSettings,
    CodexConfiguration,
    CodexRequirements,
    CodexSelection,
    DaemonSelection,
}
impl PolicyFile {
    fn path(self, environment: &Environment) -> PathBuf {
        environment.path(match self {
            Self::ClaudeSettings => "/root/.claude/settings.json",
            Self::CodexConfiguration => "/root/.codex/config.toml",
            Self::CodexRequirements => "/etc/codex/requirements.toml",
            Self::CodexSelection => "/root/.codex/packages/standalone/auto-update-version",
            Self::DaemonSelection => "/root/.codex/packages/app-server-daemon/auto-update-version",
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PolicyChange {
    file: PolicyFile,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}

pub(crate) fn plan_policy(
    environment: &Environment,
    tool: Tool,
    fresh_host_opt_in: bool,
) -> Result<Vec<PolicyChange>> {
    let files: &[PolicyFile] = match tool {
        Tool::Claude => &[PolicyFile::ClaudeSettings],
        Tool::Codex => &[
            PolicyFile::CodexConfiguration,
            PolicyFile::CodexRequirements,
            PolicyFile::CodexSelection,
            PolicyFile::DaemonSelection,
        ],
    };
    let mut changes = Vec::new();
    for file in files {
        let path = file.path(environment);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ManagerError::Rejected(
                    "needs_attention",
                    "policy configuration is an externally managed symlink",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ManagerError::Io {
                    context: "cannot inspect policy configuration",
                    source,
                });
            }
        }
        let before = read_optional(&path)?;
        let after = match file {
            PolicyFile::ClaudeSettings => {
                let mut document: Value = match &before {
                    Some(bytes) => serde_json::from_slice(bytes)?,
                    None => json!({}),
                };
                let object = document.as_object_mut().ok_or(ManagerError::Rejected(
                    "invalid_settings",
                    "Claude settings must be an object",
                ))?;
                let environment_values = object
                    .entry("env")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or(ManagerError::Rejected(
                        "invalid_settings",
                        "Claude env settings must be an object",
                    ))?;
                environment_values.insert("DISABLE_AUTOUPDATER".to_owned(), json!("1"));
                serde_json::to_vec_pretty(&document)?
            }
            PolicyFile::CodexConfiguration | PolicyFile::CodexRequirements => {
                let text = match &before {
                    Some(bytes) => std::str::from_utf8(bytes).map_err(|_| {
                        ManagerError::Rejected(
                            "invalid_settings",
                            "Codex configuration is not UTF-8",
                        )
                    })?,
                    None => "",
                };
                let mut document: toml_edit::DocumentMut = text.parse()?;
                if document
                    .get("features")
                    .is_some_and(|item| !item.is_table_like())
                {
                    return Err(ManagerError::Rejected(
                        "invalid_settings",
                        "Codex features must be a table",
                    ));
                }
                document["check_for_update_on_startup"] = toml_edit::value(false);
                document["features"]["in_app_updates"] = toml_edit::value(false);
                if matches!(file, PolicyFile::CodexConfiguration) && fresh_host_opt_in {
                    document["sandbox_mode"] = toml_edit::value("danger-full-access");
                    document["approval_policy"] = toml_edit::value("on-request");
                }
                document.to_string().into_bytes()
            }
            PolicyFile::CodexSelection | PolicyFile::DaemonSelection => {
                if before.is_none() {
                    continue;
                }
                Vec::new()
            }
        };
        changes.push(PolicyChange {
            file: *file,
            before,
            after,
        });
    }
    Ok(changes)
}

pub(crate) fn apply_policy(environment: &Environment, changes: &[PolicyChange]) -> Result<()> {
    for change in changes {
        if change.before.as_deref() == Some(change.after.as_slice()) {
            continue;
        }
        let path = change.file.path(environment);
        if read_optional(&path)? != change.before {
            return Err(ManagerError::Rejected(
                "policy_conflict",
                "configuration changed before policy application",
            ));
        }
        write_bytes(&path, &change.after)?;
    }
    Ok(())
}
pub(crate) fn rollback_policy(environment: &Environment, changes: &[PolicyChange]) -> Result<()> {
    for change in changes.iter().rev() {
        let path = change.file.path(environment);
        let current = read_optional(&path)?;
        if current == change.before {
            continue;
        }
        if current.as_deref() != Some(change.after.as_slice()) {
            return Err(ManagerError::Rejected(
                "policy_conflict",
                "policy rollback would overwrite concurrent configuration edits",
            ));
        }
        match &change.before {
            Some(bytes) => write_bytes(&path, bytes)?,
            None => fs::remove_file(&path)
                .context("cannot remove manager-created policy file during rollback")?,
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsChange {
    pub(crate) tool: Option<Tool>,
    pub(crate) enabled: Option<bool>,
    pub(crate) automatic_updates: Option<bool>,
    pub(crate) backup_interval_minutes: Option<u16>,
}

pub(crate) fn save_settings(environment: &Environment, change: SettingsChange) -> Result<()> {
    let _admission = jobs::lock_file(&environment.path("/run/ai-cli-manager/admission.lock"))?;
    if let Some(tool) = change.tool
        && jobs::active(environment, tool)?
    {
        return Err(ManagerError::Rejected(
            "busy",
            "wait for the tool job before changing its settings",
        ));
    }
    let _lock = jobs::lock_file(&environment.path("/run/ai-cli-manager/settings.lock"))?;
    let before = environment.settings()?;
    let mut settings = before.clone();
    if let Some(interval) = change.backup_interval_minutes {
        settings.backup_interval_minutes = interval;
    }
    if let Some(tool) = change.tool {
        let selected = settings.tool_mut(tool);
        if let Some(enabled) = change.enabled {
            selected.enabled = enabled;
        }
        if let Some(automatic) = change.automatic_updates {
            selected.automatic_updates = automatic;
        }
    } else if change.enabled.is_some() || change.automatic_updates.is_some() {
        return Err(ManagerError::Rejected(
            "invalid_settings",
            "select a tool before changing tool settings",
        ));
    }
    settings.validate()?;
    write_json(
        &environment.path("/boot/config/plugins/ai-cli-manager/settings.json"),
        &settings,
    )?;
    if let Err(error) = crate::unraid::reconcile_cron(environment) {
        write_json(
            &environment.path("/boot/config/plugins/ai-cli-manager/settings.json"),
            &before,
        )?;
        crate::unraid::reconcile_cron(environment)?;
        return Err(error);
    }
    Ok(())
}

pub(crate) fn activate(environment: &Environment, tool: Tool) -> Result<()> {
    let _lock = jobs::lock_file(&environment.path("/run/ai-cli-manager/settings.lock"))?;
    let mut settings: Settings = environment.settings()?;
    let selected = settings.tool_mut(tool);
    selected.initialized = true;
    selected.enabled = true;
    write_json(
        &environment.path("/boot/config/plugins/ai-cli-manager/settings.json"),
        &settings,
    )
}

pub(crate) fn restore_tool_settings(
    environment: &Environment,
    tool: Tool,
    before: &crate::domain::ToolSettings,
) -> Result<()> {
    let _lock = jobs::lock_file(&environment.path("/run/ai-cli-manager/settings.lock"))?;
    let mut settings = environment.settings()?;
    let mut activated = before.clone();
    activated.enabled = true;
    activated.initialized = true;
    if settings.tool(tool) != before && settings.tool(tool) != &activated {
        return Err(ManagerError::Rejected(
            "settings_conflict",
            "initialization rollback would overwrite changed tool settings",
        ));
    }
    *settings.tool_mut(tool) = before.clone();
    write_json(
        &environment.path("/boot/config/plugins/ai-cli-manager/settings.json"),
        &settings,
    )
}

pub(crate) fn policy_is_current(environment: &Environment, tool: Tool) -> Result<bool> {
    Ok(plan_policy(environment, tool, false)?
        .iter()
        .all(|change| change.before.as_deref() == Some(change.after.as_slice())))
}

pub(crate) fn record_policy(environment: &Environment, tool: Tool) -> Result<()> {
    write_json(
        &environment.path(format!("/root/.ai-cli-manager/policy-{}.json", tool.name())),
        &json!({"applied_at":crate::generations::timestamp()?,"processes":crate::tools::running_processes(environment,tool)?.into_iter().map(|entry|entry.0).collect::<Vec<_>>()}),
    )
}
