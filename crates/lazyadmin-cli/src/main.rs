#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre::eyre;
use futures::StreamExt;
use lazyadmin_core::{
    actions::{
        Action, ActionKind, ActionPlan, ActionResult, ActionStatus, ConfirmationPolicy, DryRunLine,
        Requirement,
    },
    config::Config,
    correlate::{EventFanIn, everything_filter},
    diff::diff_snapshots,
    doctor::{
        DoctorAdapterWatch, DoctorAdapters, DoctorCheck, DoctorEvents, DoctorReport,
        DoctorSeverity, DoctorSockets, DoctorSubsystems, DualStackProbeReport,
    },
    graph::{DiscoveryAdapter, DiscoveryContext},
    logs::{LogLine, LogOptions, LogStream, direct_unavailable},
    model::{
        ActionId, DIFF_SCHEMA_VERSION, DangerLevel, EntityRef, Process, Protocol, RuntimeKind,
        Snapshot,
    },
    selector::{Selector, parse_selector},
    snapshot::{SnapshotBuilder, build_empty_snapshot},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};
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
    Free(FreeArgs),
    Ps,
    Public,
    Conflicts,
    Projects,
    Logs(LogsArgs),
    Doctor,
    Events(EventsArgs),
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
struct FreeArgs {
    port: u16,
    #[arg(long)]
    dry_run: bool,
    #[arg(long, hide = true)]
    yes_for_test_only: bool,
}

#[derive(Args, Debug)]
struct LogsArgs {
    selector: String,
    #[arg(long)]
    tail: Option<usize>,
    #[arg(long)]
    follow: bool,
}

#[derive(Args, Debug)]
struct EventsArgs {
    #[arg(long)]
    once: bool,
    #[arg(long)]
    follow: bool,
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
                lazyadmin_tui::run_default().await.map_err(AppError::Other)
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
        Some(Command::Logs(args)) => run_logs(args, cli.json).await,
        Some(Command::Doctor) => run_doctor(cli.json).await,
        Some(Command::Events(args)) => run_events(args, cli.json).await,
        Some(Command::PauseRestart { selector }) => {
            run_pause_restart(&selector, cli.json, false).await
        }
        Some(Command::ResumeRestart { selector }) => {
            run_pause_restart(&selector, cli.json, true).await
        }
        Some(Command::Free(args)) => run_free(args, cli.json).await,
    }
}

async fn run_events(args: EventsArgs, json: bool) -> std::result::Result<(), AppError> {
    let cfg = Config::default();
    if !cfg.adapters.events.enabled {
        return unavailable("discovery events are disabled by config");
    }
    let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg);
    let procfs_stream = procfs
        .watch()
        .await
        .ok_or_else(|| AppError::Unavailable("procfs watch stream unavailable".into()))?;
    let cfg = Config::default();
    let (mut stream, _drops) = EventFanIn::new(
        vec![procfs_stream],
        cfg.adapters.events.channel_capacity,
        Duration::from_millis(250),
    );
    let event = tokio::time::timeout(
        Duration::from_secs(if args.once { 6 } else { 60 }),
        stream.next(),
    )
    .await
    .map_err(|_| AppError::Unavailable("no discovery event received before timeout".into()))?
    .ok_or_else(|| AppError::Unavailable("discovery event stream ended".into()))?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&event).map_err(|e| AppError::Other(eyre!(e)))?
        );
    } else {
        println!(
            "{:?} entity={:?} adapter={:?}",
            event.kind, event.entity, event.adapter
        );
    }
    if args.once || (!args.follow && !json) {
        return Ok(());
    }
    while let Some(event) = stream.next().await {
        if json {
            println!(
                "{}",
                serde_json::to_string(&event).map_err(|e| AppError::Other(eyre!(e)))?
            );
        } else {
            println!("{:?} {:?} {:?}", event.kind, event.entity, event.adapter);
        }
    }
    Ok(())
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

async fn run_doctor(json: bool) -> std::result::Result<(), AppError> {
    let mut checks = vec![
        check_path("sockets", "/proc/net", "/proc/net", true),
        check_path("processes", "/proc", "/proc", true),
        cmd_check(
            "sockets",
            "ss fallback",
            "ss",
            DoctorSeverity::Ok,
            DoctorSeverity::Info,
        ),
        cmd_check(
            "systemd",
            "systemctl",
            "systemctl",
            DoctorSeverity::Ok,
            DoctorSeverity::Degraded,
        ),
        cmd_check(
            "systemd",
            "journalctl",
            "journalctl",
            DoctorSeverity::Ok,
            DoctorSeverity::Degraded,
        ),
    ];
    let docker = PathBuf::from("/var/run/docker.sock");
    if docker.exists() {
        checks.push(DoctorCheck { subsystem: "containers".into(), name: "Docker socket".into(), severity: DoctorSeverity::Warning, summary: "Docker socket accessible; this usually grants root-equivalent control of the host".into(), hint: Some("Do not chmod the socket or add users to docker group blindly; use targeted action permissions.".into()) });
    } else {
        checks.push(DoctorCheck {
            subsystem: "containers".into(),
            name: "Docker socket".into(),
            severity: DoctorSeverity::Info,
            summary: "Docker socket not found".into(),
            hint: None,
        });
    }
    let reg = lazyadmin_adapter_tracked::Registry::default();
    checks.push(DoctorCheck {
        subsystem: "tracked runs".into(),
        name: "registry".into(),
        severity: if reg.ensure().is_ok() {
            DoctorSeverity::Ok
        } else {
            DoctorSeverity::Error
        },
        summary: format!("{} writable check", reg.path().display()),
        hint: None,
    });
    let pauses = pause_entries().unwrap_or_default();
    checks.push(DoctorCheck {
        subsystem: "paused restart".into(),
        name: "registry entries".into(),
        severity: if pauses.is_empty() {
            DoctorSeverity::Ok
        } else {
            DoctorSeverity::Warning
        },
        summary: format!(
            "{} paused restart entr{}",
            pauses.len(),
            if pauses.len() == 1 { "y" } else { "ies" }
        ),
        hint: Some("Run lazyadmin resume-restart <selector> to restore a recorded policy.".into()),
    });
    let cfg = Config::default();
    let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg.clone());
    let proc_out = procfs
        .discover(DiscoveryContext::default())
        .await
        .unwrap_or_default();
    let active_socket_path = if proc_out
        .warnings
        .iter()
        .any(|w| w.code == "SOCK_DIAG_DOWNGRADED")
    {
        "proc"
    } else {
        match cfg.adapters.sockets.preferred {
            lazyadmin_core::config::SocketDiscoveryPreference::Proc => "proc",
            lazyadmin_core::config::SocketDiscoveryPreference::SockDiag => "sock_diag",
            lazyadmin_core::config::SocketDiscoveryPreference::Both => "both",
        }
    };
    let parity_diff_count = proc_out
        .warnings
        .iter()
        .find(|w| w.code == "SOCK_DIAG_PARITY_DIFF")
        .and_then(|w| {
            w.message
                .split_whitespace()
                .find_map(|part| part.parse::<u64>().ok())
        })
        .unwrap_or(0);
    let dual_stack_attempted = proc_out
        .listeners
        .iter()
        .filter(|l| l.bind_addr.as_deref() == Some("::"))
        .count() as u64;
    let dual_stack_succeeded = proc_out
        .listeners
        .iter()
        .filter(|l| {
            matches!(
                l.dual_stack_state,
                lazyadmin_core::model::DualStackState::ConfirmedDualStack
                    | lazyadmin_core::model::DualStackState::ConfirmedV6Only
            )
        })
        .count() as u64;
    let dual_stack_errors = dual_stack_attempted.saturating_sub(dual_stack_succeeded);
    let container = lazyadmin_adapter_container::ContainerAdapter::new();
    let container_health = container.health().await;
    let systemd = lazyadmin_adapter_systemd::SystemdAdapter;
    let systemd_health = systemd.health().await;
    let report = DoctorReport::new(checks).with_subsystems(DoctorSubsystems {
        adapters: Some(DoctorAdapters {
            sockets: Some(DoctorSockets {
                preferred: format!("{:?}", cfg.adapters.sockets.preferred).to_ascii_lowercase(),
                active: active_socket_path.into(),
                degraded: proc_out
                    .warnings
                    .iter()
                    .any(|w| w.code == "SOCK_DIAG_DOWNGRADED"),
                parity_diff_count,
                dual_stack_probe: DualStackProbeReport {
                    supported: dual_stack_succeeded > 0,
                    attempted: dual_stack_attempted,
                    succeeded: dual_stack_succeeded,
                    errors: dual_stack_errors,
                },
            }),
        }),
        events: Some(DoctorEvents {
            enabled: cfg.adapters.events.enabled,
            per_adapter: vec![
                DoctorAdapterWatch {
                    adapter: "procfs".into(),
                    state: if cfg.adapters.events.enabled {
                        "polling"
                    } else {
                        "disabled"
                    }
                    .into(),
                    last_event_at: None,
                    dropped: 0,
                },
                DoctorAdapterWatch {
                    adapter: "container".into(),
                    state: if container_health.available {
                        "poll_only_events_deferred"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    last_event_at: None,
                    dropped: 0,
                },
                DoctorAdapterWatch {
                    adapter: "systemd".into(),
                    state: if systemd_health.available {
                        "poll_only_events_deferred"
                    } else {
                        "unavailable"
                    }
                    .into(),
                    last_event_at: None,
                    dropped: 0,
                },
            ],
            dropped: 0,
        }),
    });
    if json {
        print_json(&report)?;
    } else {
        render_doctor(&report);
    }
    Ok(())
}

fn check_path(subsystem: &str, name: &str, path: &str, required: bool) -> DoctorCheck {
    let ok = std::fs::read_dir(path).is_ok();
    DoctorCheck {
        subsystem: subsystem.into(),
        name: name.into(),
        severity: if ok {
            DoctorSeverity::Ok
        } else if required {
            DoctorSeverity::Error
        } else {
            DoctorSeverity::Info
        },
        summary: if ok {
            "readable".into()
        } else {
            "not readable".into()
        },
        hint: None,
    }
}
fn cmd_check(
    subsystem: &str,
    name: &str,
    cmd: &str,
    ok_sev: DoctorSeverity,
    miss_sev: DoctorSeverity,
) -> DoctorCheck {
    let ok = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    DoctorCheck {
        subsystem: subsystem.into(),
        name: name.into(),
        severity: if ok { ok_sev } else { miss_sev },
        summary: if ok {
            "available".into()
        } else {
            "not available".into()
        },
        hint: None,
    }
}
fn render_doctor(report: &DoctorReport) {
    let mut groups: BTreeMap<&str, Vec<&DoctorCheck>> = BTreeMap::new();
    for c in &report.checks {
        groups.entry(&c.subsystem).or_default().push(c);
    }
    for (g, cs) in groups {
        println!("{g}:");
        for c in cs {
            println!("  {}: {:?} ({})", c.name, c.severity, c.summary);
            if let Some(h) = &c.hint {
                println!("    hint: {h}");
            }
        }
    }
    if let Some(subsystems) = &report.subsystems {
        if let Some(adapters) = &subsystems.adapters {
            if let Some(sockets) = &adapters.sockets {
                println!("adapters.sockets:");
                println!(
                    "  preferred={} active={} degraded={} parity_diff_count={}",
                    sockets.preferred, sockets.active, sockets.degraded, sockets.parity_diff_count
                );
                println!(
                    "  dual_stack_probe: supported={} attempted={} succeeded={} errors={}",
                    sockets.dual_stack_probe.supported,
                    sockets.dual_stack_probe.attempted,
                    sockets.dual_stack_probe.succeeded,
                    sockets.dual_stack_probe.errors
                );
            }
        }
        if let Some(events) = &subsystems.events {
            println!("events:");
            println!("  enabled={} dropped={}", events.enabled, events.dropped);
            for adapter in &events.per_adapter {
                println!(
                    "  {}: state={} dropped={} last_event_at={}",
                    adapter.adapter,
                    adapter.state,
                    adapter.dropped,
                    adapter
                        .last_event_at
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| "never".into())
                );
            }
        }
    }
}

async fn run_logs(args: LogsArgs, json: bool) -> std::result::Result<(), AppError> {
    let options = LogOptions {
        tail: args.tail,
        follow: args.follow,
    };
    let stream = if let Some(sel) = args
        .selector
        .strip_prefix("run:")
        .or_else(|| args.selector.strip_prefix("tag:"))
    {
        match lazyadmin_adapter_tracked::logs(sel) {
            Ok(text) => LogStream {
                source: args.selector.clone(),
                lines: tail_text(&text, options.tail),
                unavailable_message: None,
            },
            Err(e) => LogStream {
                source: args.selector.clone(),
                lines: vec![],
                unavailable_message: Some(e.to_string()),
            },
        }
    } else if args.selector.starts_with("unit:") {
        let unit = args.selector.trim_start_matches("unit:");
        let mut cmd = std::process::Command::new("journalctl");
        cmd.arg("--no-pager")
            .arg("--output=short-iso")
            .arg("-u")
            .arg(unit);
        if let Some(n) = options.tail {
            cmd.arg("-n").arg(n.to_string());
        }
        if options.follow {
            cmd.arg("-f");
        }
        let out = cmd.output().map_err(|e| AppError::Other(eyre!(e)))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        LogStream {
            source: args.selector.clone(),
            lines: tail_text(&text, None),
            unavailable_message: None,
        }
    } else {
        direct_unavailable(&args.selector)
    };
    if json {
        print_json(&stream)?;
    } else if let Some(m) = stream.unavailable_message {
        println!("{m}");
    } else {
        for l in stream.lines {
            println!("[{}] {}", l.source, l.message);
        }
    }
    Ok(())
}
fn tail_text(text: &str, tail: Option<usize>) -> Vec<LogLine> {
    let lines: Vec<_> = text.lines().collect();
    let start = tail.map_or(0, |n| lines.len().saturating_sub(n));
    lines[start..]
        .iter()
        .map(|s| LogLine {
            timestamp: None,
            source: "log".into(),
            stream: None,
            message: (*s).into(),
        })
        .collect()
}

async fn run_pause_restart(
    selector: &str,
    json: bool,
    resume: bool,
) -> std::result::Result<(), AppError> {
    if resume {
        let removed = remove_pause(selector)?;
        if json {
            print_json(&serde_json::json!({"resumed": removed, "selector": selector}))?;
        } else {
            println!(
                "resume-restart {selector}: {}",
                if removed {
                    "registry entry removed; restore command should be run manually if needed"
                } else {
                    "no pause entry found"
                }
            );
        }
    } else {
        let entry = serde_json::json!({"target": selector, "runtime": "unknown", "original_restart_policy": "unknown", "operation_used": "registry_only_v0.1_runtime_override_deferred", "created_at": chrono::Utc::now(), "actor": std::env::var("USER").unwrap_or_else(|_| "unknown".into()), "restore_command": format!("lazyadmin resume-restart {selector}")});
        save_pause(selector, &entry)?;
        if json {
            print_json(&entry)?;
        } else {
            println!(
                "pause-restart recorded for {selector}; runtime mutation is conservative in v0.1 unless a verified manager executor is available"
            );
        }
    }
    Ok(())
}

async fn run_free(args: FreeArgs, json: bool) -> std::result::Result<(), AppError> {
    if args.yes_for_test_only {
        tracing::warn!("--yes-for-test-only bypass used; this flag is for automated tests only");
    }
    let before = build_snapshot().await?;
    let listeners: Vec<_> = before
        .listeners
        .iter()
        .filter(|l| l.port == Some(args.port))
        .cloned()
        .collect();
    let mut actions = Vec::new();
    for l in &listeners {
        for owner in &l.owners {
            if let EntityRef::Process(key) = owner {
                if let Some(p) = before.processes.iter().find(|p| &p.key == key) {
                    actions.push(plan_direct_process(p, args.port));
                }
            }
        }
    }
    if listeners.is_empty() {
        // Some test/dev servers can be observed as a process before the socket appears.
        // Keep this conservative: only plan direct SIGTERM when the port is explicit in cmdline.
        let needle = args.port.to_string();
        for p in before.processes.iter().filter(|p| {
            p.cmdline.iter().any(|a| a == &needle)
                && p.cmdline.iter().any(|a| a.contains("http.server"))
        }) {
            actions.push(plan_direct_process(p, args.port));
        }
    }
    let plan = ActionPlan {
        id: format!("free-{}", args.port),
        created_at: chrono::Utc::now(),
        target: format!(":{}", args.port),
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        dry_run: free_dry_run(args.port, &listeners, &actions),
        actions,
    };
    if args.dry_run || (!args.yes_for_test_only && !json) {
        render_free_plan(&plan);
        if args.dry_run {
            return Ok(());
        }
        if !confirm_free()? {
            return unavailable("free cancelled");
        }
    }
    if json && args.dry_run {
        print_json(&plan)?;
        return Ok(());
    }
    let futs = plan.actions.iter().map(execute_direct_action);
    let results = futures::future::join_all(futs).await;
    let after = build_snapshot().await?;
    let diff = diff_snapshots(&before, &after);
    let report = lazyadmin_core::actions::ActionExecutionReport {
        schema_version: "lazyadmin.action_report.v1".into(),
        plan,
        results,
        before_summary: listener_summary(args.port, &before),
        after_summary: listener_summary(args.port, &after),
        diff_summaries: diff.summaries,
    };
    if json {
        print_json(&report)?;
    } else {
        println!("Action complete.");
        for r in &report.results {
            println!("  {:?}: {}", r.status, r.message);
        }
        println!("Before: {}", report.before_summary);
        println!("After: {}", report.after_summary);
        if report.after_summary != "no listener" {
            println!(
                "Listener remains; SIGKILL is not automatic. Consider pause-restart or explicit escalation."
            );
        }
    }
    Ok(())
}

fn plan_direct_process(p: &Process, port: u16) -> Action {
    let pgid = p.pgid.unwrap_or(p.pid);
    let use_group = pgid == p.pid;
    Action {
        id: ActionId::new(format!(
            "signal-{}-{}",
            if use_group { "pgrp" } else { "pid" },
            p.pid
        )),
        label: if use_group {
            format!("Send SIGTERM to process group {pgid}")
        } else {
            format!("Send SIGTERM to PID {}", p.pid)
        },
        kind: if use_group {
            ActionKind::SignalProcessGroup
        } else {
            ActionKind::SignalPid
        },
        danger: DangerLevel::Destructive,
        requirements: vec![
            Requirement::ProcessKeyMatch { key: p.key.clone() },
            Requirement::TypedPhrase {
                phrase: "free".into(),
            },
        ],
        dry_run: vec![DryRunLine {
            summary: format!(
                "stop PID {} ({})",
                p.pid,
                p.cmdline
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "process".into())
            ),
            detail: Some(format!(
                "SIGTERM {}; port {port} expected to disappear; SIGKILL will not be used automatically",
                if use_group {
                    format!("process group {pgid}")
                } else {
                    format!("PID {}", p.pid)
                }
            )),
        }],
        target: EntityRef::Process(p.key.clone()),
        runtime: RuntimeKind::Direct,
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        timeout_ms: 5_000,
        provenance: vec!["procfs listener owner".into()],
    }
}
fn free_dry_run(
    port: u16,
    listeners: &[lazyadmin_core::model::Listener],
    actions: &[Action],
) -> Vec<DryRunLine> {
    let mut v = vec![DryRunLine { summary: format!("free port {port}: {} listener(s), {} owner action(s)", listeners.len(), actions.len()), detail: Some("one consolidated confirmation; manager actions would be preferred over raw signals when discovered".into()) }];
    for a in actions {
        v.extend(a.dry_run.clone());
    }
    v.push(DryRunLine {
        summary: "will not touch unrelated ports or use SIGKILL automatically".into(),
        detail: None,
    });
    v
}
fn render_free_plan(plan: &ActionPlan) {
    println!("Dry run for {}", plan.target);
    for l in &plan.dry_run {
        println!(
            "  - {}{}",
            l.summary,
            l.detail
                .as_ref()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default()
        );
    }
    println!("{}", plan.confirmation.render_prompt());
}
fn confirm_free() -> std::result::Result<bool, AppError> {
    let mut s = String::new();
    std::io::stdin()
        .read_line(&mut s)
        .map_err(|e| AppError::Other(eyre!(e)))?;
    Ok(s.trim() == "free")
}
async fn execute_direct_action(action: &Action) -> ActionResult {
    let start = Instant::now();
    let span = tracing::info_span!("action.execute", action.kind=?action.kind, target=?action.target, runtime=?action.runtime, danger=?action.danger);
    let _g = span.enter();
    let EntityRef::Process(key) = &action.target else {
        return result(
            action,
            ActionStatus::Unsupported,
            "unsupported target",
            start,
            Some("unsupported"),
        );
    };
    let snap = match build_snapshot().await {
        Ok(s) => s,
        Err(e) => {
            return result(
                action,
                ActionStatus::Failed,
                &format!("validation scan failed: {e}"),
                start,
                Some("validation"),
            );
        }
    };
    let Some(proc_) = snap.processes.iter().find(|p| &p.key == key) else {
        return result(
            action,
            ActionStatus::Skipped,
            "process already gone before signal",
            start,
            None,
        );
    };
    if &proc_.key != key {
        return result(
            action,
            ActionStatus::Failed,
            "ProcessKey mismatch; refusing to signal reused PID",
            start,
            Some("pid_reuse_guard"),
        );
    }
    let pgid = proc_.pgid.unwrap_or(proc_.pid);
    let raw_target = if matches!(action.kind, ActionKind::SignalProcessGroup) && pgid == proc_.pid {
        -pgid
    } else {
        proc_.pid
    };
    let sig_target = nix::unistd::Pid::from_raw(raw_target);
    match nix::sys::signal::kill(sig_target, nix::sys::signal::Signal::SIGTERM) {
        Ok(()) => {}
        Err(e) => {
            return result(
                action,
                ActionStatus::Failed,
                &format!("SIGTERM failed: {e}"),
                start,
                Some("signal"),
            );
        }
    }
    tokio::time::sleep(Duration::from_millis(action.timeout_ms.min(5_000))).await;
    let after = build_snapshot().await.ok();
    let gone = after
        .as_ref()
        .is_none_or(|s| !s.processes.iter().any(|p| p.key == *key));
    result(
        action,
        if gone {
            ActionStatus::Success
        } else {
            ActionStatus::TimedOut
        },
        if gone {
            "SIGTERM sent and process disappeared"
        } else {
            "SIGTERM sent; process still present after timeout"
        },
        start,
        if gone { None } else { Some("timeout") },
    )
}
fn result(
    action: &Action,
    status: ActionStatus,
    msg: &str,
    start: Instant,
    err: Option<&str>,
) -> ActionResult {
    ActionResult {
        action_id: action.id.clone(),
        status,
        message: msg.into(),
        duration_ms: start.elapsed().as_millis(),
        error_class: err.map(str::to_string),
    }
}
fn listener_summary(port: u16, snap: &Snapshot) -> String {
    let xs: Vec<_> = snap
        .listeners
        .iter()
        .filter(|l| l.port == Some(port))
        .map(|l| {
            format!(
                "{:?} {}:{} owners={}",
                l.protocol,
                l.bind_addr.as_deref().unwrap_or("*"),
                port,
                l.owners.len()
            )
        })
        .collect();
    if xs.is_empty() {
        "no listener".into()
    } else {
        xs.join("; ")
    }
}

fn pause_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lazyadmin/pauses")
}
fn save_pause(selector: &str, v: &serde_json::Value) -> std::result::Result<(), AppError> {
    let d = pause_dir();
    std::fs::create_dir_all(&d).map_err(|e| AppError::Other(eyre!(e)))?;
    let name = selector.replace(['/', ':', ' '], "_");
    std::fs::write(
        d.join(format!("{name}.json")),
        serde_json::to_vec_pretty(v).unwrap(),
    )
    .map_err(|e| AppError::Other(eyre!(e)))
}
fn pause_entries() -> std::result::Result<Vec<serde_json::Value>, AppError> {
    let d = pause_dir();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                if let Ok(v) = serde_json::from_str(&t) {
                    out.push(v);
                }
            }
        }
    }
    Ok(out)
}
fn remove_pause(selector: &str) -> std::result::Result<bool, AppError> {
    let path = pause_dir().join(format!("{}.json", selector.replace(['/', ':', ' '], "_")));
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| AppError::Other(eyre!(e)))?;
        Ok(true)
    } else {
        Ok(false)
    }
}
