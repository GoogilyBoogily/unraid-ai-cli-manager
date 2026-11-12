use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

pub(crate) type Result<T> = std::result::Result<T, ManagerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AttemptOutcome {
    NotAttempted,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JobState {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SafeFailure {
    pub(crate) code: String,
    #[serde(rename = "error")]
    pub(crate) message: String,
}
impl From<&ManagerError> for SafeFailure {
    fn from(error: &ManagerError) -> Self {
        Self {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperationOutcome {
    pub(crate) installation: AttemptOutcome,
    pub(crate) persistence: AttemptOutcome,
    pub(crate) details: serde_json::Value,
    pub(crate) failure: Option<SafeFailure>,
}
impl OperationOutcome {
    pub(crate) fn persisted(details: serde_json::Value) -> Self {
        Self {
            installation: AttemptOutcome::NotAttempted,
            persistence: AttemptOutcome::Succeeded,
            details,
            failure: None,
        }
    }
    pub(crate) fn unchanged(details: serde_json::Value) -> Self {
        Self {
            installation: AttemptOutcome::NotAttempted,
            persistence: AttemptOutcome::NotAttempted,
            details,
            failure: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ManagerError {
    #[error("{1}")]
    Rejected(&'static str, &'static str),
    #[error("{context}")]
    Io {
        context: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("invalid JSON document")]
    Json(#[from] serde_json::Error),
    #[error("SQLite backup or validation failed")]
    Database(#[from] rusqlite::Error),
    #[error("invalid TOML configuration")]
    Toml(#[from] toml_edit::TomlError),
    #[error("{operation}; rollback failed: {rollback}")]
    Rollback {
        operation: Box<ManagerError>,
        rollback: Box<ManagerError>,
    },
}

impl ManagerError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Rejected(code, _) => code,
            Self::Io { .. } => "io_failure",
            Self::Json(_) | Self::Toml(_) => "invalid_data",
            Self::Database(_) => "database_failure",
            Self::Rollback { .. } => "rollback_failed",
        }
    }
    pub(crate) fn after_rollback(self, rollback: Result<()>) -> Self {
        match rollback {
            Ok(()) => self,
            Err(rollback) => Self::Rollback {
                operation: Box::new(self),
                rollback: Box::new(rollback),
            },
        }
    }
}

pub(crate) trait Context<T> {
    fn context(self, context: &'static str) -> Result<T>;
}

impl<T> Context<T> for io::Result<T> {
    fn context(self, context: &'static str) -> Result<T> {
        self.map_err(|source| ManagerError::Io { context, source })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub(crate) const ALL: [Self; 2] = [Self::Claude, Self::Codex];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
    pub(crate) fn legacy_name(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Codex => "codex",
        }
    }
    pub(crate) fn live(self) -> &'static str {
        match self {
            Self::Claude => "/root/.claude",
            Self::Codex => "/root/.codex",
        }
    }
    pub(crate) fn persistent(self) -> &'static str {
        match self {
            Self::Claude => "/mnt/user/appdata/claude-code",
            Self::Codex => "/mnt/user/appdata/codex",
        }
    }
    pub(crate) fn marker(self) -> String {
        format!("/root/.{}-persist-restored", self.name())
    }
    pub(crate) fn lock(self) -> String {
        format!("/var/run/{}-persist.lock", self.name())
    }
    pub(crate) fn launcher(self) -> PathBuf {
        PathBuf::from(format!("/root/.local/bin/{}", self.name()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ToolSelection {
    Claude,
    Codex,
    All,
}
impl ToolSelection {
    pub(crate) fn tools(self) -> Vec<Tool> {
        match self {
            Self::Claude => vec![Tool::Claude],
            Self::Codex => vec![Tool::Codex],
            Self::All => Tool::ALL.to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Initialize,
    Restore,
    Backup,
    Install,
    Update,
    #[cfg(feature = "test-support")]
    Probe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolSettings {
    pub(crate) enabled: bool,
    pub(crate) initialized: bool,
    pub(crate) automatic_updates: bool,
}
impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            initialized: false,
            automatic_updates: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) backup_interval_minutes: u16,
    pub(crate) claude: ToolSettings,
    pub(crate) codex: ToolSettings,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            backup_interval_minutes: 15,
            claude: ToolSettings::default(),
            codex: ToolSettings::default(),
        }
    }
}
impl Settings {
    pub(crate) fn tool(&self, tool: Tool) -> &ToolSettings {
        match tool {
            Tool::Claude => &self.claude,
            Tool::Codex => &self.codex,
        }
    }
    pub(crate) fn tool_mut(&mut self, tool: Tool) -> &mut ToolSettings {
        match tool {
            Tool::Claude => &mut self.claude,
            Tool::Codex => &mut self.codex,
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if ![5, 15, 30, 60].contains(&self.backup_interval_minutes) {
            return Err(ManagerError::Rejected(
                "invalid_settings",
                "backup interval must be 5, 15, 30, or 60",
            ));
        }
        if Tool::ALL
            .iter()
            .any(|tool| self.tool(*tool).enabled && !self.tool(*tool).initialized)
        {
            return Err(ManagerError::Rejected(
                "initialization_required",
                "initialize a tool before enabling it",
            ));
        }
        Ok(())
    }
}
