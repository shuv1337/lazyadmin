#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre::eyre;
use lazyadmin_core::{
    config::Config,
    correlate::everything_filter,
    diff::diff_snapshots,
    graph::{DiscoveryAdapter, DiscoveryContext},
    model::{DIFF_SCHEMA_VERSION, Protocol, Snapshot},
    selector::{Selector, parse_selector},
    snapshot::{SnapshotBuilder, build_empty_snapshot},
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
    Run(RunArgs),
    Runs {
        #[arg(long)]
        json: bool,
    },
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

#[derive(Args, Debug)]
struct RunArgs {
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    detach: bool,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env")]
    envs: Vec<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    cmd: Vec<String>,
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
                run_point_query(&selector, cli.json, cli.brief).await
            } else {
                unavailable("TUI is not implemented yet")
            }
        }
        Some(Command::Export) => {
            let snap = build_snapshot().await?;
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
        Some(Command::Port { port }) => {
            run_point_query(&format!(":{port}"), cli.json, cli.brief).await
        }
        Some(Command::Run(args)) => run_run(args, cli.json).await,
        Some(Command::Runs { json }) => run_runs(cli.json || json).await,
        Some(Command::Ps) => run_view("ps", cli.json, cli.brief).await,
        Some(Command::Public) => run_view("public", cli.json, cli.brief).await,
        Some(Command::Conflicts) => run_view("conflicts", cli.json, cli.brief).await,
        Some(Command::Projects) => run_view("projects", cli.json, cli.brief).await,
        Some(Command::Logs { .. })
        | Some(Command::Doctor)
        | Some(Command::PauseRestart { .. })
        | Some(Command::ResumeRestart { .. })
        | Some(Command::Free { .. }) => unavailable("mutating/log commands are deferred"),
    }
}

fn unavailable<T>(msg: impl Into<String>) -> std::result::Result<T, AppError> {
    Err(AppError::Unavailable(msg.into()))
}

async fn build_snapshot() -> std::result::Result<Snapshot, AppError> {
    let cfg = Config::default();
    let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg.clone());
    let tracked = lazyadmin_adapter_tracked::TrackedAdapter::new();
    let systemd = lazyadmin_adapter_systemd::SystemdAdapter;
    let container = lazyadmin_adapter_container::ContainerAdapter::new();
    let project = lazyadmin_adapter_project::ProjectAdapter::new(cfg);
    let mut outputs = Vec::new();
    outputs.push(
        procfs
            .discover(DiscoveryContext::default())
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?,
    );
    outputs.push(
        tracked
            .discover(DiscoveryContext::default())
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?,
    );
    outputs.push(
        systemd
            .discover(DiscoveryContext::default())
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?,
    );
    outputs.push(
        container
            .discover(DiscoveryContext::default())
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?,
    );
    outputs.push(
        project
            .discover(DiscoveryContext::default())
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?,
    );
    let mut snap = SnapshotBuilder::from_adapter_outputs(outputs);
    let runs = lazyadmin_adapter_tracked::Registry::default()
        .list()
        .unwrap_or_default();
    for run in runs {
        if let Some(pid) = run.pid {
            for process in &mut snap.processes {
                if process.pid == pid as i32 {
                    process.lazyadmin_run_id =
                        Some(lazyadmin_core::model::RunId::new(run.id.clone()));
                }
            }
        }
    }
    Ok(snap)
}

async fn run_view(kind: &str, json: bool, brief: bool) -> std::result::Result<(), AppError> {
    let mut snap = build_snapshot().await?;
    let hidden = everything_filter(&snap, &Config::default()).hidden_count;
    match kind {
        "public" => snap.listeners.retain(|l| {
            l.exposure != lazyadmin_core::model::Exposure::Loopback
                && l.exposure != lazyadmin_core::model::Exposure::UnixLocal
        }),
        "conflicts" => {
            let ids: std::collections::HashSet<_> = snap
                .warnings
                .iter()
                .filter(|w| w.code == "CONFLICT")
                .filter_map(|w| match &w.entity {
                    Some(lazyadmin_core::model::EntityRef::Listener(id)) => Some(id.clone()),
                    _ => None,
                })
                .collect();
            snap.listeners
                .retain(|l| ids.contains(&l.id) || l.owners.len() > 1);
        }
        "projects" => {
            snap.listeners.clear();
            snap.processes.clear();
        }
        _ => {}
    }
    if json {
        print_json(&snap)?;
        return Ok(());
    }
    if kind == "projects" {
        for p in &snap.projects {
            println!(
                "{} {} markers={}",
                p.name,
                p.root.display(),
                p.markers.len()
            );
        }
        return Ok(());
    }
    if !brief && hidden > 0 {
        println!("{hidden} system workloads hidden by default view");
    }
    for l in &snap.listeners {
        println!(
            "{:?} {}:{} owners={}",
            l.protocol,
            l.bind_addr.as_deref().unwrap_or("*"),
            l.port.unwrap_or(0),
            l.owners.len()
        );
    }
    Ok(())
}

async fn run_point_query(
    selector: &str,
    json: bool,
    brief: bool,
) -> std::result::Result<(), AppError> {
    let sel = parse_selector(selector).map_err(|e| AppError::Other(eyre!(e)))?;
    let snap = build_snapshot().await?;
    let Selector::Socket(sock) = sel else {
        if json {
            print_json(&snap)?;
            return Ok(());
        }
        println!(
            "point query matched selector {selector}; detailed non-socket inspector is minimal in v0.1"
        );
        return Ok(());
    };
    let mut filtered = snap.clone();
    filtered.listeners = snap
        .listeners
        .into_iter()
        .filter(|l| {
            l.port == Some(sock.port)
                && (sock.protocol == Protocol::Any || l.protocol == sock.protocol)
        })
        .collect();
    if json {
        print_json(&filtered)?;
        return Ok(());
    }
    if filtered.listeners.is_empty() {
        println!("no listener found on :{}", sock.port);
        return Ok(());
    }
    for l in &filtered.listeners {
        if brief {
            println!(
                "{} {}:{} owners={}",
                format!("{:?}", l.protocol).to_lowercase(),
                l.bind_addr.as_deref().unwrap_or("*"),
                l.port.unwrap_or(0),
                l.owners.len()
            );
        } else {
            println!(
                "listener {:?} {}:{} inode {:?} confidence {:?}",
                l.protocol,
                l.bind_addr.as_deref().unwrap_or("*"),
                l.port.unwrap_or(0),
                l.socket_inode,
                l.confidence
            );
            for o in &l.owners {
                println!("  owner: {o:?}");
            }
            for w in filtered.warnings.iter().filter(|w| {
                w.entity.as_ref().is_some_and(|e| {
                    l.owners.contains(e)
                        || *e == lazyadmin_core::model::EntityRef::Listener(l.id.clone())
                })
            }) {
                println!("  warning {}: {}", w.code, w.message);
            }
        }
    }
    Ok(())
}

async fn run_runs(json: bool) -> std::result::Result<(), AppError> {
    let reg = lazyadmin_adapter_tracked::Registry::default();
    let runs = reg.list().map_err(|e| AppError::Other(eyre!(e)))?;
    if json {
        print_json(&serde_json::json!({"tracked_runs": runs}))?;
    } else {
        for r in runs {
            println!("{} {:?} {:?}", r.id, r.tag, r.state);
        }
    }
    Ok(())
}

async fn run_run(args: RunArgs, json: bool) -> std::result::Result<(), AppError> {
    if let Some(action) = args.cmd.first().cloned() {
        if matches!(action.as_str(), "stop" | "logs" | "forget" | "restart") {
            let sel = args
                .cmd
                .get(1)
                .ok_or_else(|| AppError::Other(eyre!("selector required")))?;
            return match action.as_str() {
                "stop" => {
                    if lazyadmin_adapter_tracked::stop(sel)
                        .map_err(|e| AppError::Other(eyre!(e)))?
                    {
                        println!("stopped {sel}");
                        Ok(())
                    } else {
                        unavailable("run not found")
                    }
                }
                "logs" => {
                    print!(
                        "{}",
                        lazyadmin_adapter_tracked::logs(sel)
                            .map_err(|e| AppError::Other(eyre!(e)))?
                    );
                    Ok(())
                }
                "forget" => {
                    if lazyadmin_adapter_tracked::forget(sel)
                        .map_err(|e| AppError::Other(eyre!(e)))?
                    {
                        println!("forgot {sel}");
                        Ok(())
                    } else {
                        unavailable("run not found")
                    }
                }
                "restart" => unavailable(
                    "run restart is deferred: direct MVP does not restore process trees safely",
                ),
                _ => unreachable!(),
            };
        }
    }
    if !args.detach {
        return unavailable("only --detach is implemented for lazyadmin run MVP");
    }
    let envs = args
        .envs
        .into_iter()
        .filter_map(|e| {
            e.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    let cmd = if args.cmd.first().is_some_and(|s| s == "--") {
        args.cmd[1..].to_vec()
    } else {
        args.cmd
    };
    let entry = lazyadmin_adapter_tracked::spawn_detached(args.tag, args.cwd, envs, cmd)
        .map_err(|e| AppError::Other(eyre!(e)))?;
    if json {
        print_json(&entry)?;
    } else {
        println!(
            "started {} pid {:?} log_source={}",
            entry.id, entry.pid, entry.log_source
        );
    }
    Ok(())
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
