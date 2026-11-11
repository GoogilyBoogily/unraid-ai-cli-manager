#![forbid(unsafe_code)]

mod domain;
mod environment;
mod jobs;
mod process;

use clap::{Parser, Subcommand};
use domain::{ManagerError, Operation, Result, Tool, ToolSelection};
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
    #[command(name = "__internal", hide = true)]
    Internal { #[command(subcommand)] action: InternalAction },
}

#[derive(Subcommand)]
enum InternalAction {
    Supervise { #[arg(long)] identifier: String },
    Worker { #[arg(long)] identifier: String },
    Job { #[arg(long)] identifier: String },
    Initialize { #[arg(long, value_enum)] tool: Tool, #[arg(long)] host_execution_opt_in: bool },
    #[cfg(feature = "test-support")]
    Probe { #[arg(long, value_enum)] tool: Tool, #[arg(long, default_value_t = 500)] milliseconds: u64 },
}

fn execute_operation(environment: &Environment, _tool: Tool, operation: Operation, _consent: bool, deadline: std::time::Instant) -> Result<Value> {
    #[cfg(feature = "test-support")]
    if operation == Operation::Probe {
        process::run_checked(environment, "probe", &[], deadline)?;
        return Ok(json!({"installation":"not_attempted", "persistence":"not_attempted"}));
    }
    let _ = (environment, operation, deadline);
    Err(ManagerError::Rejected("initialization_required", "tool operation is not initialized"))
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
        Action::Internal { action } => match action {
            InternalAction::Supervise { identifier } => jobs::supervise(&environment, &identifier),
            InternalAction::Worker { identifier } => jobs::worker(&environment, &identifier),
            InternalAction::Job { identifier } => Ok(serde_json::to_value(jobs::read_job(&environment, &identifier)?)?),
            InternalAction::Initialize { tool, host_execution_opt_in } => jobs::submit(&environment, tool, Operation::Initialize, host_execution_opt_in, 600_000),
            #[cfg(feature = "test-support")]
            InternalAction::Probe { tool, milliseconds } => jobs::submit(&environment, tool, Operation::Probe, false, milliseconds),
        },
        action => {
            let (selection, operation) = match action {
                Action::Restore { tool, .. } => (tool, Operation::Restore),
                Action::Backup { tool, .. } => (tool, Operation::Backup),
                Action::Install { tool, .. } => (tool, Operation::Install),
                Action::Update { tool, .. } => (tool, Operation::Update),
                _ => unreachable!("status and private actions already matched"),
            };
            let reports: Vec<Value> = selection.tools().into_iter().map(|tool| match jobs::submit(&environment, tool, operation, false, 600_000) { Ok(receipt) => json!({"tool":tool,"receipt":receipt}), Err(error) => json!({"tool":tool,"code":error.code(),"error":error.to_string()}) }).collect();
            if reports.iter().all(|report| report.get("receipt").is_none()) { return Err(ManagerError::Rejected("job_rejected", "no selected tool accepted the operation")); }
            Ok(json!({"jobs":reports}))
        }
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

