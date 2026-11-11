#![forbid(unsafe_code)]

mod domain;
mod environment;

use clap::{Parser, Subcommand};
use domain::{ManagerError, Result, ToolSelection};
use environment::Environment;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(version, about = "Host-native AI CLI persistence for Unraid")]
struct Arguments {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Status { #[arg(long, value_enum, default_value = "all")] tool: ToolSelection, #[arg(long)] json: bool },
    Restore { #[arg(long, value_enum)] tool: ToolSelection, #[arg(long)] json: bool },
    Backup { #[arg(long, value_enum)] tool: ToolSelection, #[arg(long)] json: bool },
    Install { #[arg(long, value_enum)] tool: ToolSelection, #[arg(long)] json: bool },
    Update { #[arg(long, value_enum)] tool: ToolSelection, #[arg(long)] json: bool },
}

fn read_status(environment: &Environment, selection: ToolSelection) -> Result<Value> {
    let settings = environment.settings()?;
    let storage = environment.storage_available()?;
    let mut tools = serde_json::Map::new();
    for tool in selection.tools() {
        let setting = settings.tool(tool);
        let readiness = if !setting.enabled { "disabled" } else if !storage { "waiting_for_storage" } else if !environment.path(tool.marker()).is_file() { "needs_attention" } else { "ready" };
        tools.insert(tool.name().to_owned(), json!({"readiness": readiness, "operation": "idle", "restart_required": false, "versions": {"installed": null, "cached": null, "daemon": null}, "last_successful_backup": null, "last_successful_update": null, "errors": []}));
    }
    Ok(json!({"schema_version": 1, "tools": tools}))
}

fn run(arguments: Arguments) -> Result<Value> {
    let environment = Environment::from_process()?;
    match arguments.command {
        Action::Status { tool, .. } => read_status(&environment, tool),
        _ => Err(ManagerError::Rejected("initialization_required", "initialize the selected tool before requesting work")),
    }
}

fn main() {
    let arguments = Arguments::parse();
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    match run(arguments) {
        Ok(value) => println!("{value}"),
        Err(error) => { eprintln!("{}", json!({"code": error.code(), "error": error.to_string()})); std::process::exit(1); }
    }
}

