#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre::eyre;
use lazyadmin_core::{
    config::Config,
    diff::diff_snapshots,
    model::{DIFF_SCHEMA_VERSION, Snapshot},
    snapshot::build_empty_snapshot,
};
use std::{path::PathBuf, process::ExitCode};
use tracing::{error, info_span};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan, prelude::*};

const EX_OK: u8 = 0;
const EX_UNAVAILABLE: u8 = 78;

#[derive(Parser, Debug)]
#[command(name = "lazyadmin", version, about = "Local runtime control plane")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    brief: bool,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value_t = LogFormat::Text)]
    log_format: LogFormat,
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(value_name = "SELECTOR", hide = true)]
    selector: Option<String>,
}

#[derive(Clone, Debug, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
enum Command {
    Port {
        port: u16,
    },
    Free {
        port: u16,
    },
    Ps,
    Public,
    Conflicts,
    Projects,
    Logs {
        selector: String,
    },
    Doctor,
    Export,
    Diff(DiffArgs),
    Run {
        #[arg(trailing_var_arg = true)]
        cmd: Vec<String>,
    },
    Runs,
    PauseRestart {
        selector: String,
    },
    ResumeRestart {
        selector: String,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    Check,
}

#[derive(Args, Debug)]
struct DiffArgs {
    before: PathBuf,
    after: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = color_eyre::install() {
        eprintln!("failed to initialize color-eyre: {e}");
    }
    let cli = Cli::parse();
    init_tracing(&cli);
    match run(cli).await {
        Ok(()) => ExitCode::from(EX_OK),
        Err(AppError::Unavailable(msg)) => {
            eprintln!("unavailable: {msg}");
            ExitCode::from(EX_UNAVAILABLE)
        }
        Err(AppError::Other(err)) => {
            error!(error.class = %err, "command failed");
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn init_tracing(cli: &Cli) {
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let registry = tracing_subscriber::registry().with(filter);
    match cli.log_format {
        LogFormat::Text => {
            let layer = tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::CLOSE)
                .with_writer(std::io::stderr);
            let _ = registry.with(layer).try_init();
        }
        LogFormat::Json => {
            let layer = tracing_subscriber::fmt::layer()
                .json()
                .with_span_events(FmtSpan::CLOSE)
                .with_writer(std::io::stderr);
            let _ = registry.with(layer).try_init();
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Other(#[from] color_eyre::Report),
}

async fn run(cli: Cli) -> std::result::Result<(), AppError> {
    let span = info_span!("cli.command", json = cli.json, brief = cli.brief);
    let _guard = span.enter();
    match cli.command {
        None => {
            if let Some(selector) = cli.selector {
                unavailable(format!("point query for {selector} is not implemented yet"))
            } else {
                unavailable("TUI is not implemented yet")
            }
        }
        Some(Command::Export) => {
            let snap = build_empty_snapshot();
            print_json(&snap)?;
            Ok(())
        }
        Some(Command::Diff(args)) => run_diff(args, cli.json).await,
        Some(Command::Config {
            command: ConfigCommand::Check,
        }) => {
            let cfg = Config::load(cli.config.as_deref()).map_err(|e| AppError::Other(eyre!(e)))?;
            if cli.json {
                print_json(&serde_json::json!({"ok": true, "config": cfg}))?;
            } else {
                println!("config ok");
            }
            Ok(())
        }
        Some(Command::Port { .. })
        | Some(Command::Free { .. })
        | Some(Command::Ps)
        | Some(Command::Public)
        | Some(Command::Conflicts)
        | Some(Command::Projects)
        | Some(Command::Logs { .. })
        | Some(Command::Doctor)
        | Some(Command::Run { .. })
        | Some(Command::Runs)
        | Some(Command::PauseRestart { .. })
        | Some(Command::ResumeRestart { .. }) => {
            unavailable("command is not implemented in PLAN-01 foundation")
        }
    }
}

fn unavailable<T>(msg: impl Into<String>) -> std::result::Result<T, AppError> {
    Err(AppError::Unavailable(msg.into()))
}

async fn run_diff(args: DiffArgs, json: bool) -> std::result::Result<(), AppError> {
    let before = read_snapshot(&args.before)?;
    let after = if args.after.as_os_str() == "-" {
        build_empty_snapshot()
    } else {
        read_snapshot(&args.after)?
    };
    let diff = diff_snapshots(&before, &after);
    if json {
        print_json(&diff)?;
    } else {
        println!("Diff ({DIFF_SCHEMA_VERSION})");
        for s in diff.summaries {
            println!("{s}");
        }
    }
    Ok(())
}

fn read_snapshot(path: &PathBuf) -> std::result::Result<Snapshot, AppError> {
    if path.as_os_str() == "-" {
        let snap: Snapshot =
            serde_json::from_reader(std::io::stdin()).map_err(|e| AppError::Other(eyre!(e)))?;
        Ok(snap)
    } else {
        let file = std::fs::File::open(path).map_err(|e| AppError::Other(eyre!(e)))?;
        serde_json::from_reader(file).map_err(|e| AppError::Other(eyre!(e)))
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> std::result::Result<(), AppError> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|e| AppError::Other(eyre!(e)))?
    );
    Ok(())
}
