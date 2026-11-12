#![forbid(unsafe_code)]

mod configuration;
mod domain;
mod environment;
mod generations;
mod jobs;
mod migration;
mod operations;
mod process;
mod sqlite;
mod tools;
mod unraid;

use clap::{Parser, Subcommand};
use domain::{ManagerError, Operation, OperationOutcome, Result, Tool, ToolSelection};
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
    Status {
        #[arg(long, value_enum, default_value = "all")]
        tool: ToolSelection,
        #[arg(long)]
        json: bool,
    },
    Restore {
        #[arg(long, value_enum)]
        tool: ToolSelection,
        #[arg(long)]
        json: bool,
    },
    Backup {
        #[arg(long, value_enum)]
        tool: ToolSelection,
        #[arg(long)]
        json: bool,
    },
    Install {
        #[arg(long, value_enum)]
        tool: ToolSelection,
        #[arg(long)]
        json: bool,
    },
    Update {
        #[arg(long, value_enum)]
        tool: ToolSelection,
        #[arg(long)]
        json: bool,
    },
    #[command(name = "__internal", hide = true)]
    Internal {
        #[command(subcommand)]
        action: InternalAction,
    },
}

#[derive(Subcommand)]
enum InternalAction {
    Supervise {
        #[arg(long)]
        identifier: String,
    },
    Worker {
        #[arg(long)]
        identifier: String,
    },
    Job {
        #[arg(long)]
        identifier: String,
    },
    Initialize {
        #[arg(long, value_enum)]
        tool: Tool,
        #[arg(long)]
        host_execution_opt_in: bool,
    },
    Settings,
    Tick {
        #[arg(long, value_enum)]
        operation: ScheduledOperation,
        #[arg(long, value_enum)]
        tool: Tool,
    },
    Event {
        event: String,
    },
    Start,
    Shutdown {
        #[arg(long, default_value_t = 120_000)]
        budget_milliseconds: u64,
    },
    Uninstall,
    PluginHook {
        kind: String,
        method: String,
        plugin: String,
        error: String,
    },
    #[cfg(feature = "test-support")]
    Probe {
        #[arg(long, value_enum)]
        tool: Tool,
        #[arg(long, default_value_t = 500)]
        milliseconds: u64,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ScheduledOperation {
    Backup,
    Update,
}

fn execute_operation(
    environment: &Environment,
    tool: Tool,
    operation: Operation,
    consent: bool,
    deadline: std::time::Instant,
) -> Result<OperationOutcome> {
    operations::execute(environment, tool, operation, consent, deadline)
}

fn run(arguments: Arguments) -> Result<Value> {
    let environment = Environment::from_process()?;
    match arguments.command {
        Action::Status { tool, .. } => operations::status(&environment, tool),
        Action::Internal { action } => match action {
            InternalAction::Tick { operation, tool } => jobs::scheduled(
                &environment,
                tool,
                match operation {
                    ScheduledOperation::Backup => Operation::Backup,
                    ScheduledOperation::Update => Operation::Update,
                },
            ),
            InternalAction::Event { event } => unraid::event(&environment, &event),
            InternalAction::Start => unraid::start(&environment),
            InternalAction::Shutdown {
                budget_milliseconds,
            } => unraid::shutdown(&environment, budget_milliseconds, false),
            InternalAction::Uninstall => unraid::shutdown(&environment, 120_000, true),
            InternalAction::PluginHook {
                kind,
                method,
                plugin,
                error,
            } => unraid::plugin_hook(&environment, &kind, &method, &plugin, &error),
            InternalAction::Settings => {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::io::stdin()
                    .take(16_385)
                    .read_to_end(&mut bytes)
                    .map_err(|source| ManagerError::Io {
                        context: "cannot read settings request",
                        source,
                    })?;
                if bytes.len() > 16_384 {
                    return Err(ManagerError::Rejected(
                        "invalid_settings",
                        "settings request is too large",
                    ));
                }
                configuration::save_settings(&environment, serde_json::from_slice(&bytes)?)?;
                Ok(json!({"saved":true}))
            }
            InternalAction::Supervise { identifier } => jobs::supervise(&environment, &identifier),
            InternalAction::Worker { identifier } => Ok(serde_json::to_value(jobs::worker(
                &environment,
                &identifier,
            )?)?),
            InternalAction::Job { identifier } => Ok(serde_json::to_value(jobs::read_job(
                &environment,
                &identifier,
            )?)?),
            InternalAction::Initialize {
                tool,
                host_execution_opt_in,
            } => jobs::submit(
                &environment,
                tool,
                Operation::Initialize,
                host_execution_opt_in,
                600_000,
            ),
            #[cfg(feature = "test-support")]
            InternalAction::Probe { tool, milliseconds } => {
                jobs::submit(&environment, tool, Operation::Probe, false, milliseconds)
            }
        },
        action => {
            let (selection, operation) = match action {
                Action::Restore { tool, .. } => (tool, Operation::Restore),
                Action::Backup { tool, .. } => (tool, Operation::Backup),
                Action::Install { tool, .. } => (tool, Operation::Install),
                Action::Update { tool, .. } => (tool, Operation::Update),
                _ => unreachable!("status and private actions already matched"),
            };
            let tools = selection.tools();
            if tools.len() == 1 {
                let tool = tools[0];
                let receipt = jobs::submit(&environment, tool, operation, false, 600_000)?;
                return Ok(json!({"jobs":[{"tool":tool,"receipt":receipt}]}));
            }
            let reports: Vec<Value> = tools
                .into_iter()
                .map(
                    |tool| match jobs::submit(&environment, tool, operation, false, 600_000) {
                        Ok(receipt) => json!({"tool":tool,"receipt":receipt}),
                        Err(error) => {
                            json!({"tool":tool,"code":error.code(),"error":error.to_string()})
                        }
                    },
                )
                .collect();
            let accepted = reports.iter().any(|report| report.get("receipt").is_some());
            Ok(json!({"jobs":reports,"accepted":accepted}))
        }
    }
}

fn main() {
    let arguments = Arguments::parse();
    rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));
    match run(arguments) {
        Ok(value) => {
            println!("{value}");
            if value["accepted"] == false {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!(
                "{}",
                json!({"code": error.code(), "error": error.to_string()})
            );
            std::process::exit(1);
        }
    }
}
