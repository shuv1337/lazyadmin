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
    correlate::{EventDropCounter, EventFanIn, everything_filter},
    diff::diff_snapshots,
    doctor::{
        DoctorAdapterWatch, DoctorAdapters, DoctorCheck, DoctorEvents, DoctorReport,
        DoctorSeverity, DoctorSockets, DoctorSubsystems, DualStackProbeReport,
    },
    graph::{DiscoveryAdapter, DiscoveryContext},
    logs::{LogLine, LogOptions, LogStream, direct_unavailable},
    model::{
        ActionId, DIFF_SCHEMA_VERSION, DangerLevel, EntityRef, Listener, Process, Protocol,
        RuntimeKind, Snapshot, Workload,
    },
    output::listener_rows,
    selector::{Selector, parse_selector},
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
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
    Tui(TuiArgs),
    Web(WebArgs),
    Port {
        port: u16,
    },
    Free(FreeArgs),
    Ps,
    Public,
    Conflicts,
    Projects,
    Logs(LogsArgs),
    Doctor(DoctorArgs),
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

#[derive(Args, Debug)]
struct TuiArgs {
    #[arg(long)]
    headless: bool,
    #[arg(long)]
    theme: Option<String>,
}

#[derive(Args, Debug)]
struct WebArgs {
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,
    #[arg(long, default_value_t = 7749)]
    port: u16,
    #[arg(long)]
    no_open: bool,
    #[arg(long, default_value_t = 2000)]
    refresh_ms: u64,
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
struct DoctorArgs {
    #[arg(long)]
    groups: bool,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    actionable: bool,
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
                run_point_query(&selector, cli.json, cli.brief, cli.config.as_deref()).await
            } else {
                run_tui_command(
                    TuiArgs {
                        headless: false,
                        theme: None,
                    },
                    cli.json,
                    cli.config.as_deref(),
                )
                .await
            }
        }
        Some(Command::Export) => {
            let snap = build_snapshot(cli.config.as_deref()).await?;
            print_json(&snap)?;
            Ok(())
        }
        Some(Command::Tui(args)) => run_tui_command(args, cli.json, cli.config.as_deref()).await,
        Some(Command::Web(args)) => run_web(args, cli.config).await,
        Some(Command::Diff(args)) => run_diff(args, cli.json, cli.config.as_deref()).await,
        Some(Command::Config {
            command: ConfigCommand::Check,
        }) => {
            let cfg = Config::load(cli.config.as_deref()).map_err(|e| AppError::Other(eyre!(e)))?;
            validate_tui_theme_config(&cfg).map_err(|e| AppError::Other(eyre!(e)))?;
            if cli.json {
                let keybindings =
                    lazyadmin_core::config::keybindings::ResolvedKeybindings::from_config(&cfg)
                        .map_err(|e| AppError::Other(eyre!(e)))?;
                print_json(
                    &serde_json::json!({"ok": true, "config": cfg, "keybindings": keybindings}),
                )?;
            } else {
                println!("config ok");
            }
            Ok(())
        }
        Some(Command::Port { port }) => {
            run_point_query(
                &format!(":{port}"),
                cli.json,
                cli.brief,
                cli.config.as_deref(),
            )
            .await
        }
        Some(Command::Run(args)) => run_run(args, cli.json).await,
        Some(Command::Runs { json }) => run_runs(cli.json || json).await,
        Some(Command::Ps) => run_view("ps", cli.json, cli.brief, cli.config.as_deref()).await,
        Some(Command::Public) => {
            run_view("public", cli.json, cli.brief, cli.config.as_deref()).await
        }
        Some(Command::Conflicts) => {
            run_view("conflicts", cli.json, cli.brief, cli.config.as_deref()).await
        }
        Some(Command::Projects) => {
            run_view("projects", cli.json, cli.brief, cli.config.as_deref()).await
        }
        Some(Command::Logs(args)) => run_logs(args, cli.json).await,
        Some(Command::Doctor(args)) => run_doctor(args, cli.json, cli.config.as_deref()).await,
        Some(Command::Events(args)) => run_events(args, cli.json, cli.config.as_deref()).await,
        Some(Command::PauseRestart { selector }) => {
            run_pause_restart(&selector, cli.json, false).await
        }
        Some(Command::ResumeRestart { selector }) => {
            run_pause_restart(&selector, cli.json, true).await
        }
        Some(Command::Free(args)) => run_free(args, cli.json).await,
    }
}

async fn run_tui_command(
    args: TuiArgs,
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let cfg = Config::load(config_path).map_err(|e| AppError::Other(eyre!(e)))?;
    let snap = build_snapshot(config_path).await?;
    let theme_name = args.theme.as_deref().or(cfg.ui.theme.name.as_deref());
    let (theme, hint) = lazyadmin_tui::Theme::load(theme_name, cfg.ui.theme.path.as_deref())
        .map_err(|e| AppError::Other(eyre!(e)))?
        .downgrade_for_colors(lazyadmin_tui::detected_color_count());
    let keybindings = lazyadmin_core::config::keybindings::ResolvedKeybindings::from_config(&cfg)
        .map_err(|e| AppError::Other(eyre!(e)))?;
    if args.headless {
        let dump = lazyadmin_tui::headless_dump(&snap, 120, theme, keybindings);
        if json {
            print_json(&dump)?;
        } else {
            println!(
                "lazyadmin TUI headless: {:?} panes={}",
                dump.layout.mode,
                dump.panes.len()
            );
        }
        return Ok(());
    }
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(4);
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
    spawn_tui_refresh_task(
        cfg.clone(),
        config_path.map(std::path::Path::to_path_buf),
        snapshot_tx,
        event_tx,
    );
    let runtime = lazyadmin_tui::TuiRuntime {
        initial_snapshot: snap,
        config: lazyadmin_tui::AppConfig {
            refresh_interval: Duration::from_millis(cfg.ui.refresh.tick_ms),
            show_system: false,
            event_debounce: Duration::from_millis(cfg.ui.refresh.event_debounce_ms),
            max_redraw_hz: cfg.ui.refresh.max_redraw_hz,
        },
        theme,
        keybindings,
        color_hint: hint,
        allow_open_non_loopback: cfg.actions.open_non_loopback,
        snapshots: Some(snapshot_rx),
        discovery_events: Some(event_rx),
        config_reload: Some(Box::new({
            let config_path = config_path.map(std::path::Path::to_path_buf);
            move || {
                let cfg = Config::load(config_path.as_deref())?;
                let theme = lazyadmin_tui::Theme::load(
                    cfg.ui.theme.name.as_deref(),
                    cfg.ui.theme.path.as_deref(),
                )?
                .downgrade_for_colors(lazyadmin_tui::detected_color_count())
                .0;
                let keybindings =
                    lazyadmin_core::config::keybindings::ResolvedKeybindings::from_config(&cfg)?;
                Ok((theme, keybindings))
            }
        })),
    };
    lazyadmin_tui::run_tui_with_runtime(runtime)
        .await
        .map_err(AppError::Other)
}

fn validate_tui_theme_config(cfg: &Config) -> anyhow::Result<()> {
    lazyadmin_tui::Theme::load(cfg.ui.theme.name.as_deref(), cfg.ui.theme.path.as_deref())?;
    Ok(())
}

async fn run_web(args: WebArgs, config_path: Option<PathBuf>) -> std::result::Result<(), AppError> {
    let options = lazyadmin_web::WebOptions {
        bind: args.bind,
        port: args.port,
        config_path,
        refresh_interval: Duration::from_millis(args.refresh_ms),
    };
    let (info, handle) = lazyadmin_web::bind(options)
        .await
        .map_err(|e| AppError::Other(eyre!(e)))?;
    println!("lazyadmin web listening on {}", info.url);
    if !args.no_open {
        // Auto-open is intentionally a no-op in v1; --no-open is reserved for
        // future behavior so smoke tests and scripts can opt out preemptively.
        eprintln!("(browser auto-open is not implemented in v1; open the URL manually)");
    }
    handle
        .await
        .map_err(|e| AppError::Other(eyre!(e)))?
        .map_err(|e| AppError::Other(eyre!(e)))
}

async fn event_streams_for_config(
    cfg: &Config,
) -> Vec<futures::stream::BoxStream<'static, lazyadmin_core::model::DiscoveryEvent>> {
    lazyadmin_runtime::event_streams_for_config(cfg).await
}

fn spawn_tui_refresh_task(
    cfg: Config,
    config_path: Option<PathBuf>,
    snapshot_tx: tokio::sync::mpsc::Sender<Snapshot>,
    event_tx: tokio::sync::mpsc::Sender<lazyadmin_core::model::DiscoveryEvent>,
) {
    tokio::spawn(async move {
        let streams = event_streams_for_config(&cfg).await;
        let has_events = !streams.is_empty();
        let (mut events, drops) = EventFanIn::new(
            streams,
            cfg.adapters.events.channel_capacity,
            Duration::from_millis(cfg.ui.refresh.event_debounce_ms),
        );
        let mut interval = tokio::time::interval(Duration::from_millis(cfg.ui.refresh.tick_ms));
        if !has_events {
            loop {
                interval.tick().await;
                match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                    Ok(snapshot) => {
                        if snapshot_tx.send(snapshot).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => tracing::debug!(error = %err, "tui snapshot refresh failed"),
                }
            }
            return;
        }
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                        Ok(snapshot) => {
                            if snapshot_tx.send(snapshot).await.is_err() { break; }
                        }
                        Err(err) => tracing::debug!(error = %err, "tui snapshot refresh failed"),
                    }
                }
                event = events.next() => {
                    match event {
                        Some(event) => {
                            let _ = event_tx.send(event).await;
                            tokio::time::sleep(Duration::from_millis(cfg.ui.refresh.event_debounce_ms)).await;
                            match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                                Ok(snapshot) => {
                                    if snapshot_tx.send(snapshot).await.is_err() { break; }
                                }
                                Err(err) => tracing::debug!(error = %err, "tui event snapshot refresh failed"),
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });
}

async fn run_events(
    args: EventsArgs,
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let cfg = Config::load(config_path).map_err(|e| AppError::Other(eyre!(e)))?;
    if !cfg.adapters.events.enabled {
        return unavailable("discovery events are disabled by config");
    }
    let streams = event_streams_for_config(&cfg).await;
    if streams.is_empty() {
        return unavailable("no discovery event streams are available");
    }
    let (mut stream, _drops) = EventFanIn::new(
        streams,
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

async fn build_snapshot(
    config_path: Option<&std::path::Path>,
) -> std::result::Result<Snapshot, AppError> {
    build_snapshot_with_event_drops(config_path, None).await
}

async fn build_snapshot_with_event_drops(
    config_path: Option<&std::path::Path>,
    event_drops: Option<&EventDropCounter>,
) -> std::result::Result<Snapshot, AppError> {
    lazyadmin_runtime::build_snapshot_with_event_drops(config_path, event_drops)
        .await
        .map_err(|e| AppError::Other(eyre!(e)))
}

async fn run_view(
    kind: &str,
    json: bool,
    brief: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let cfg = Config::load(config_path).map_err(|e| AppError::Other(eyre!(e)))?;
    let mut snap = build_snapshot(config_path).await?;
    let hidden = everything_filter(&snap, &cfg).hidden_count;
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
    let rows = listener_rows(&snap);
    for row in &rows {
        let manager = row
            .manager_detail
            .as_ref()
            .map(|detail| format!(" ({detail})"))
            .unwrap_or_default();
        println!(
            "{:?} {}:{} owners={}{}",
            row.protocol,
            row.bind_addr.as_deref().unwrap_or("*"),
            row.port.unwrap_or(0),
            row.owners_count,
            manager
        );
    }
    Ok(())
}

async fn run_point_query(
    selector: &str,
    json: bool,
    brief: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let sel = parse_selector(selector).map_err(|e| AppError::Other(eyre!(e)))?;
    let snap = build_snapshot(config_path).await?;
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

async fn run_diff(
    args: DiffArgs,
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let before = read_snapshot(&args.before)?;
    let after = read_diff_after_snapshot(&args.after, config_path).await?;
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

async fn read_diff_after_snapshot(
    path: &PathBuf,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<Snapshot, AppError> {
    if path.as_os_str() == "-" {
        build_snapshot(config_path).await
    } else {
        read_snapshot(path)
    }
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

async fn run_doctor(
    args: DoctorArgs,
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    if args.groups {
        let snapshot = lazyadmin_runtime::build_snapshot(config_path)
            .await
            .map_err(|e| AppError::Other(eyre!(e)))?;
        let groups = lazyadmin_runtime::view_model::build_doctor_groups(&snapshot);
        if json {
            print_json(&groups)?;
        } else {
            render_doctor_groups(&groups, args.all, args.actionable);
        }
        return Ok(());
    }
    let cfg = Config::load(config_path).map_err(|e| AppError::Other(eyre!(e)))?;
    let report = build_doctor_report(cfg, None).await;
    if json {
        print_json(&report)?;
    } else {
        render_doctor(&report);
    }
    Ok(())
}

async fn build_doctor_report(cfg: Config, event_drops: Option<&EventDropCounter>) -> DoctorReport {
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
    let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg.clone());
    let proc_out = procfs
        .discover(DiscoveryContext::default())
        .await
        .unwrap_or_default();
    checks.extend(portless_doctor_checks(&proc_out));
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
    let systemd =
        lazyadmin_adapter_systemd::SystemdAdapter::new(cfg.adapters.systemd.events_enabled);
    let systemd_health = systemd.health().await;
    let observable_event_drops = event_drops.map(EventDropCounter::dropped);
    DoctorReport::new(checks).with_subsystems(DoctorSubsystems {
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
        events: Some({
            let dropped = observable_event_drops.unwrap_or(0);
            DoctorEvents {
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
                        dropped,
                    },
                    DoctorAdapterWatch {
                        adapter: "container".into(),
                        state: if !cfg.adapters.container.events_enabled {
                            "disabled"
                        } else if container_health.available && container.capabilities().watching {
                            "docker_events"
                        } else {
                            "unavailable"
                        }
                        .into(),
                        last_event_at: None,
                        dropped: 0,
                    },
                    DoctorAdapterWatch {
                        adapter: "systemd".into(),
                        state: if !cfg.adapters.systemd.events_enabled {
                            "disabled"
                        } else if systemd_health.available && systemd.capabilities().watching {
                            "dbus_signals"
                        } else {
                            "unavailable"
                        }
                        .into(),
                        last_event_at: None,
                        dropped: 0,
                    },
                ],
                dropped,
                drop_counter_observable: observable_event_drops.is_some(),
                drop_counter_source: Some(if observable_event_drops.is_some() {
                    "shared_event_fan_in".into()
                } else {
                    "unavailable_in_stateless_cli_doctor".into()
                }),
            }
        }),
    })
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

fn portless_doctor_checks(proc_out: &lazyadmin_core::graph::DiscoveryOutput) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let state_dirs = lazyadmin_adapter_portless::default_state_dirs();
    if state_dirs.is_empty() {
        checks.push(DoctorCheck {
            subsystem: "adapter:portless".into(),
            name: "state dir".into(),
            severity: DoctorSeverity::Info,
            summary: "no PORTLESS_STATE_DIR or HOME state directory resolved".into(),
            hint: None,
        });
    }
    let mut total_orphans = 0usize;
    for state_dir in &state_dirs {
        let routes_path = state_dir.join("routes.json");
        if !state_dir.exists() {
            checks.push(DoctorCheck {
                subsystem: "adapter:portless".into(),
                name: format!("state dir {}", state_dir.display()),
                severity: DoctorSeverity::Info,
                summary: "not present".into(),
                hint: None,
            });
            continue;
        }
        match std::fs::read(&routes_path) {
            Ok(bytes) => {
                let mut warnings = Vec::new();
                let routes =
                    lazyadmin_adapter_portless::parse_routes(&bytes, state_dir, &mut warnings);
                checks.push(DoctorCheck {
                    subsystem: "adapter:portless".into(),
                    name: format!("state dir {}", state_dir.display()),
                    severity: if warnings
                        .iter()
                        .any(|warning| warning.code == "portless.routes_unparseable")
                    {
                        DoctorSeverity::Warning
                    } else {
                        DoctorSeverity::Ok
                    },
                    summary: format!(
                        "{} route(s) readable from {}",
                        routes.len(),
                        routes_path.display()
                    ),
                    hint: warnings
                        .iter()
                        .find(|warning| warning.code == "portless.routes_unparseable")
                        .map(|warning| warning.message.clone()),
                });
                total_orphans += routes
                    .iter()
                    .filter(|route| route.pid != 0 && !pid_alive(route.pid))
                    .count();
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => checks.push(DoctorCheck {
                subsystem: "adapter:portless".into(),
                name: format!("state dir {}", state_dir.display()),
                severity: DoctorSeverity::Info,
                summary: format!("{} not found", routes_path.display()),
                hint: None,
            }),
            Err(err) => checks.push(DoctorCheck {
                subsystem: "adapter:portless".into(),
                name: format!("state dir {}", state_dir.display()),
                severity: DoctorSeverity::Warning,
                summary: format!("{} unreadable: {err}", routes_path.display()),
                hint: Some("Check file permissions; lazyadmin only reads portless state.".into()),
            }),
        }

        let lock = state_dir.join("routes.lock");
        if let Ok(metadata) = std::fs::metadata(&lock) {
            let age = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .unwrap_or_default();
            checks.push(DoctorCheck {
                subsystem: "adapter:portless".into(),
                name: format!("routes.lock {}", state_dir.display()),
                severity: if age > Duration::from_secs(30) {
                    DoctorSeverity::Warning
                } else {
                    DoctorSeverity::Ok
                },
                summary: format!("lock age {}s", age.as_secs()),
                hint: (age > Duration::from_secs(30)).then_some(
                    "portless uses a mkdir lock with a 10s stale threshold; investigate stuck writers."
                        .into(),
                ),
            });
        }

        checks.extend(portless_proxy_checks(state_dir, proc_out));
    }
    checks.push(DoctorCheck {
        subsystem: "adapter:portless".into(),
        name: "binary".into(),
        severity: if command_available("portless") {
            DoctorSeverity::Ok
        } else {
            DoctorSeverity::Info
        },
        summary: if command_available("portless") {
            portless_version_summary()
        } else {
            "portless not found on PATH".into()
        },
        hint: None,
    });
    checks.push(DoctorCheck {
        subsystem: "adapter:portless".into(),
        name: "orphan routes".into(),
        severity: if total_orphans > 0 {
            DoctorSeverity::Info
        } else {
            DoctorSeverity::Ok
        },
        summary: format!("{total_orphans} orphaned route(s)"),
        hint: (total_orphans > 0).then_some(format!(
            "run `portless prune` to clean up {total_orphans} orphaned route(s)"
        )),
    });
    checks
}

fn portless_proxy_checks(
    state_dir: &std::path::Path,
    proc_out: &lazyadmin_core::graph::DiscoveryOutput,
) -> Vec<DoctorCheck> {
    let pidfile = state_dir.join("proxy.pid");
    if !pidfile.exists() {
        return Vec::new();
    }
    let pid = std::fs::read_to_string(&pidfile)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());
    let port = std::fs::read_to_string(state_dir.join("proxy.port"))
        .ok()
        .and_then(|text| text.trim().parse::<u16>().ok());
    let alive = pid.is_some_and(pid_alive);
    let listening = match (pid, port) {
        (Some(pid), Some(port)) => proc_out.listeners.iter().any(|listener| {
            listener.port == Some(port)
                && listener.owners.iter().any(|owner| match owner {
                    EntityRef::Process(key) => key.pid == pid,
                    _ => false,
                })
        }),
        (Some(_), None) => true,
        _ => false,
    };
    vec![DoctorCheck {
        subsystem: "adapter:portless".into(),
        name: format!("proxy daemon {}", state_dir.display()),
        severity: if alive && listening {
            DoctorSeverity::Ok
        } else {
            DoctorSeverity::Warning
        },
        summary: format!(
            "pid={} port={} alive={} listening={}",
            pid.map_or_else(|| "unknown".into(), |pid| pid.to_string()),
            port.map_or_else(|| "unknown".into(), |port| port.to_string()),
            alive,
            listening
        ),
        hint: (!alive).then_some("proxy pidfile points at a dead process".into()),
    }]
}

fn pid_alive(pid: i32) -> bool {
    std::path::Path::new("/proc").join(pid.to_string()).exists()
}

fn command_available(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn portless_version_summary() -> String {
    std::process::Command::new("portless")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|version| !version.is_empty())
        .map(|version| format!("available ({version})"))
        .unwrap_or_else(|| "available".into())
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
            println!(
                "  {} {} ({})",
                severity_badge(&c.severity),
                c.name,
                c.summary
            );
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
            println!(
                "  enabled={} dropped={} drop_counter_observable={} source={}",
                events.enabled,
                events.dropped,
                events.drop_counter_observable,
                events.drop_counter_source.as_deref().unwrap_or("unknown")
            );
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

fn render_doctor_groups(
    view: &lazyadmin_runtime::view_model::DoctorGroupsView,
    show_all: bool,
    actionable_only: bool,
) {
    println!(
        "doctor: {} actionable warning(s), {} noise group(s), {} noise warning(s)",
        view.actionable_count, view.noise_group_count, view.noise_total_count
    );
    if view.groups.is_empty() {
        println!("Everything's clean — no actionable warnings.");
        return;
    }
    for group in &view.groups {
        let is_noise = matches!(group.tier, lazyadmin_core::doctor::WarningTier::Noise);
        if actionable_only && is_noise {
            continue;
        }
        println!(
            "  [{:?}] {:?} {} ×{} — {}",
            group.tier, group.severity, group.label, group.count, group.remediation
        );
        if show_all || group.expanded {
            for entity in &group.sample_entities {
                println!("    sample: {entity:?}");
            }
        } else if is_noise {
            println!("    collapsed noise; pass --all to show samples");
        }
    }
}

fn severity_badge(severity: &DoctorSeverity) -> &'static str {
    match severity {
        DoctorSeverity::Ok => "\x1b[32m[OK]\x1b[0m",
        DoctorSeverity::Info => "\x1b[36m[INFO]\x1b[0m",
        DoctorSeverity::Warning => "\x1b[33m[WARN]\x1b[0m",
        DoctorSeverity::Degraded => "\x1b[35m[DEGRADED]\x1b[0m",
        DoctorSeverity::Error => "\x1b[31m[ERROR]\x1b[0m",
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
    let before = build_snapshot(None).await?;
    let parts = plan_free_for_snapshot(&before, args.port, true);
    let mut actions = parts.actions();
    let plan = ActionPlan {
        id: format!("free-{}", args.port),
        created_at: chrono::Utc::now(),
        target: format!(":{}", args.port),
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        dry_run: free_dry_run(args.port, &parts.listeners, &actions),
        actions: actions.clone(),
    };
    if args.dry_run {
        if json {
            print_json(&plan)?;
        } else {
            render_free_plan(&plan);
        }
        return Ok(());
    }
    if !args.yes_for_test_only && !json {
        render_free_plan(&plan);
        if !confirm_free()? {
            return unavailable("free cancelled");
        }
    }
    let mut results = Vec::new();
    if !parts.portless_actions.is_empty() {
        results.extend(
            futures::future::join_all(parts.portless_actions.iter().map(execute_portless_stop))
                .await,
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let direct_snapshot = if parts.portless_actions.is_empty() {
        before.clone()
    } else {
        build_snapshot(None).await?
    };
    let direct_parts = plan_free_for_snapshot(&direct_snapshot, args.port, false);
    actions = parts.portless_actions.clone();
    actions.extend(direct_parts.direct_actions.clone());
    results.extend(
        futures::future::join_all(
            direct_parts
                .direct_actions
                .iter()
                .map(execute_direct_action),
        )
        .await,
    );
    let after = build_snapshot(None).await?;
    let diff = diff_snapshots(&before, &after);
    let report = lazyadmin_core::actions::ActionExecutionReport {
        schema_version: "lazyadmin.action_report.v1".into(),
        plan: ActionPlan { actions, ..plan },
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
                "Listener remains; SIGKILL is not automatic. Consider pause-restart, lazyadmin doctor, or explicit escalation."
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct FreePlanParts {
    listeners: Vec<Listener>,
    portless_actions: Vec<Action>,
    direct_actions: Vec<Action>,
}

impl FreePlanParts {
    fn actions(&self) -> Vec<Action> {
        self.portless_actions
            .iter()
            .chain(self.direct_actions.iter())
            .cloned()
            .collect()
    }
}

fn plan_free_for_snapshot(snap: &Snapshot, port: u16, include_portless: bool) -> FreePlanParts {
    let listeners: Vec<_> = snap
        .listeners
        .iter()
        .filter(|listener| listener.port == Some(port))
        .cloned()
        .collect();
    let mut parts = FreePlanParts {
        listeners,
        ..FreePlanParts::default()
    };
    let mut planned_processes = std::collections::HashSet::new();
    let mut planned_portless = std::collections::BTreeSet::new();
    for listener in &parts.listeners {
        let portless_workloads = portless_workloads_for_listener(snap, listener);
        if include_portless {
            for workload in &portless_workloads {
                if planned_portless.insert(workload.id.clone()) {
                    if let Some(action) = plan_portless_stop(workload, port) {
                        parts.portless_actions.push(action);
                    }
                }
            }
        }
        if !portless_workloads.is_empty() {
            continue;
        }
        for owner in &listener.owners {
            if let EntityRef::Process(key) = owner {
                if planned_processes.insert(key.clone()) {
                    if let Some(process) = snap.processes.iter().find(|process| &process.key == key)
                    {
                        parts
                            .direct_actions
                            .push(plan_direct_process(process, port));
                    }
                }
            }
        }
    }
    if parts.listeners.is_empty() && include_portless {
        // Some test/dev servers can be observed as a process before the socket appears.
        // Keep this conservative: only plan direct SIGTERM when the port is explicit in cmdline.
        let needle = port.to_string();
        for process in snap.processes.iter().filter(|process| {
            process.cmdline.iter().any(|arg| arg == &needle)
                && process
                    .cmdline
                    .iter()
                    .any(|arg| arg.contains("http.server"))
        }) {
            if planned_processes.insert(process.key.clone()) {
                parts
                    .direct_actions
                    .push(plan_direct_process(process, port));
            }
        }
    }
    parts
}

fn portless_workloads_for_listener<'a>(
    snap: &'a Snapshot,
    listener: &Listener,
) -> Vec<&'a Workload> {
    let listener_ref = EntityRef::Listener(listener.id.clone());
    snap.edges
        .iter()
        .filter(|edge| {
            edge.kind == lazyadmin_core::model::EdgeKind::WorkloadOwnsListener
                && edge.to == listener_ref
        })
        .filter_map(|edge| match &edge.from {
            EntityRef::Workload(id) => snap
                .workloads
                .iter()
                .find(|workload| &workload.id == id && workload.runtime == RuntimeKind::Portless),
            _ => None,
        })
        .collect()
}

fn plan_portless_stop(workload: &Workload, port: u16) -> Option<Action> {
    let Some(EntityRef::Process(key)) = &workload.source else {
        return None;
    };
    Some(Action {
        id: ActionId::new(format!("portless-stop-{}", workload.id)),
        label: format!("Stop portless app {}", workload.display_name),
        kind: ActionKind::PortlessStop,
        danger: DangerLevel::Destructive,
        requirements: vec![
            Requirement::ProcessKeyMatch { key: key.clone() },
            Requirement::TypedPhrase {
                phrase: "free".into(),
            },
        ],
        dry_run: vec![DryRunLine {
            summary: format!(
                "stop portless app \"{}\" (manager: portless)",
                workload.display_name
            ),
            detail: Some(format!(
                "SIGTERM PID {} (portless cli); portless will killTree the dev-server and remove the route for port {port}",
                key.pid
            )),
        }],
        target: EntityRef::Process(key.clone()),
        runtime: RuntimeKind::Portless,
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        timeout_ms: 5_000,
        provenance: vec![format!("portless workload {}", workload.id)],
    })
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
    let mut v = vec![DryRunLine { summary: format!("free port {port}: {} listener(s), {} owner action(s)", listeners.len(), actions.len()), detail: Some("one consolidated confirmation; portless routes are stopped through their CLI, direct owners use process-key guarded SIGTERM".into()) }];
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
    let snap = match build_snapshot(None).await {
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
    let after = build_snapshot(None).await.ok();
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

async fn execute_portless_stop(action: &Action) -> ActionResult {
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
    let snap = match build_snapshot(None).await {
        Ok(snapshot) => snapshot,
        Err(err) => {
            return result(
                action,
                ActionStatus::Failed,
                &format!("validation scan failed: {err}"),
                start,
                Some("validation"),
            );
        }
    };
    let Some(proc_) = snap.processes.iter().find(|process| &process.key == key) else {
        return result(
            action,
            ActionStatus::Skipped,
            "portless CLI already gone before signal",
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
    match nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(proc_.pid),
        nix::sys::signal::Signal::SIGTERM,
    ) {
        Ok(()) => {}
        Err(err) => {
            return result(
                action,
                ActionStatus::Failed,
                &format!("SIGTERM failed: {err}"),
                start,
                Some("signal"),
            );
        }
    }
    tokio::time::sleep(Duration::from_millis(action.timeout_ms.min(5_000))).await;
    let after = build_snapshot(None).await.ok();
    let gone = after
        .as_ref()
        .is_none_or(|snapshot| !snapshot.processes.iter().any(|process| process.key == *key));
    result(
        action,
        if gone {
            ActionStatus::Success
        } else {
            ActionStatus::TimedOut
        },
        if gone {
            "SIGTERM sent to portless CLI and process disappeared"
        } else {
            "SIGTERM sent to portless CLI; process still present after timeout"
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

#[cfg(test)]
mod tests {
    use super::*;
    use lazyadmin_core::model::{
        AddressFamily, Confidence, DualStackState, Edge, EdgeKind, Exposure, ListenerId,
        ListenerState, ProcessKey, RedactedEnvironmentSummary, WorkloadId, WorkloadState,
    };

    #[tokio::test]
    async fn doctor_report_honors_socket_and_event_config() {
        let mut cfg = Config::default();
        cfg.adapters.sockets.preferred = lazyadmin_core::config::SocketDiscoveryPreference::Both;
        cfg.adapters.events.enabled = false;

        let report = build_doctor_report(cfg, None).await;
        let subsystems = report.subsystems.unwrap();
        let sockets = subsystems.adapters.unwrap().sockets.unwrap();
        assert_eq!(sockets.preferred, "both");
        let events = subsystems.events.unwrap();
        assert!(!events.enabled);
        assert!(!events.drop_counter_observable);
        assert_eq!(
            events.drop_counter_source.as_deref(),
            Some("unavailable_in_stateless_cli_doctor")
        );
        assert_eq!(
            events
                .per_adapter
                .iter()
                .find(|adapter| adapter.adapter == "procfs")
                .unwrap()
                .state,
            "disabled"
        );
    }

    #[tokio::test]
    async fn doctor_json_contract_stays_flat_without_group_fields() {
        let report = build_doctor_report(Config::default(), None).await;
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("checks").is_some());
        assert!(json.get("groups").is_none());
        assert!(json.get("actionable_count").is_none());

        let snapshot = lazyadmin_core::snapshot::build_empty_snapshot();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json.get("warnings").is_some());
        assert!(json.get("doctor_groups").is_none());
    }

    #[tokio::test]
    async fn doctor_report_uses_shared_event_drop_counter_when_available() {
        let (mut fan_in, drops) = EventFanIn::new(vec![], 1, Duration::from_millis(0));
        fan_in.push_event_for_test(lazyadmin_core::model::DiscoveryEvent::heartbeat("a"));
        fan_in.push_event_for_test(lazyadmin_core::model::DiscoveryEvent::heartbeat("b"));

        let report = build_doctor_report(Config::default(), Some(&drops)).await;
        let events = report.subsystems.unwrap().events.unwrap();
        assert_eq!(events.dropped, 1);
        assert!(events.drop_counter_observable);
        assert_eq!(
            events.drop_counter_source.as_deref(),
            Some("shared_event_fan_in")
        );
        assert_eq!(
            events
                .per_adapter
                .iter()
                .find(|adapter| adapter.adapter == "procfs")
                .unwrap()
                .dropped,
            1
        );
    }

    #[tokio::test]
    async fn diff_dash_after_uses_current_snapshot() {
        let snap = read_diff_after_snapshot(&PathBuf::from("-"), None)
            .await
            .unwrap();
        assert!(!snap.managers.is_empty());
    }

    #[test]
    fn free_planner_prefers_portless_cli_over_descendant_owner() {
        let cli = process_key(100, 10);
        let child = process_key(101, 11);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(100), vec!["node"]));
        let listener_id = ListenerId::new("tcp:127.0.0.1:3737:1");
        snap.listeners.push(listener(
            listener_id.clone(),
            3737,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:demo");
        snap.workloads.push(Workload {
            id: workload_id.clone(),
            display_name: "demo.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![listener_id.clone()],
            project: None,
            manager: None,
            source: Some(EntityRef::Process(cli.clone())),
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(listener_id),
            provenance: vec![],
        });

        let plan = plan_free_for_snapshot(&snap, 3737, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert!(plan.direct_actions.is_empty());
        assert_eq!(plan.portless_actions[0].kind, ActionKind::PortlessStop);
        assert_eq!(plan.portless_actions[0].target, EntityRef::Process(cli));
    }

    #[test]
    fn free_planner_handles_direct_and_mixed_ports() {
        let direct = process_key(200, 20);
        let cli = process_key(300, 30);
        let child = process_key(301, 31);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(direct.clone(), None, vec!["python"]));
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(300), vec!["node"]));
        let direct_listener = ListenerId::new("tcp:127.0.0.1:8080:1");
        let portless_listener = ListenerId::new("tcp:127.0.0.1:8080:2");
        snap.listeners.push(listener(
            direct_listener,
            8080,
            vec![EntityRef::Process(direct.clone())],
        ));
        snap.listeners.push(listener(
            portless_listener.clone(),
            8080,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:mixed");
        snap.workloads.push(Workload {
            id: workload_id.clone(),
            display_name: "mixed.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![portless_listener.clone()],
            project: None,
            manager: None,
            source: Some(EntityRef::Process(cli.clone())),
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(portless_listener),
            provenance: vec![],
        });

        let plan = plan_free_for_snapshot(&snap, 8080, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert_eq!(plan.direct_actions.len(), 1);
        assert_eq!(plan.portless_actions[0].target, EntityRef::Process(cli));
        assert_eq!(plan.direct_actions[0].target, EntityRef::Process(direct));
    }

    #[test]
    fn free_planner_dedupes_same_portless_workload() {
        let cli = process_key(400, 40);
        let child = process_key(401, 41);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(400), vec!["node"]));
        let workload_id = WorkloadId::new("portless:dedupe");
        snap.workloads.push(Workload {
            id: workload_id.clone(),
            display_name: "dedupe.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![],
            project: None,
            manager: None,
            source: Some(EntityRef::Process(cli)),
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        for suffix in [1, 2] {
            let listener_id = ListenerId::new(format!("tcp:127.0.0.1:9090:{suffix}"));
            snap.listeners.push(listener(
                listener_id.clone(),
                9090,
                vec![EntityRef::Process(child.clone())],
            ));
            snap.edges.push(Edge {
                kind: EdgeKind::WorkloadOwnsListener,
                from: EntityRef::Workload(workload_id.clone()),
                to: EntityRef::Listener(listener_id),
                provenance: vec![],
            });
        }

        let plan = plan_free_for_snapshot(&snap, 9090, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert!(plan.direct_actions.is_empty());
    }

    #[test]
    fn free_planner_refuses_portless_without_source_and_ignores_alias_without_listener() {
        let child = process_key(501, 51);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(child.clone(), None, vec!["node"]));
        let listener_id = ListenerId::new("tcp:127.0.0.1:6060:1");
        snap.listeners.push(listener(
            listener_id.clone(),
            6060,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:missing-source");
        snap.workloads.push(Workload {
            id: workload_id.clone(),
            display_name: "missing-source.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![listener_id.clone()],
            project: None,
            manager: None,
            source: None,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        snap.workloads.push(Workload {
            id: WorkloadId::new("portless:alias"),
            display_name: "alias.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![],
            project: None,
            manager: None,
            source: None,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(listener_id),
            provenance: vec![],
        });

        let plan = plan_free_for_snapshot(&snap, 6060, true);
        assert!(plan.portless_actions.is_empty());
        assert!(plan.direct_actions.is_empty());
    }

    fn process_key(pid: i32, start_time_ticks: u64) -> ProcessKey {
        ProcessKey {
            pid,
            boot_id: "boot".into(),
            start_time_ticks,
        }
    }

    fn process(key: ProcessKey, ppid: Option<i32>, cmdline: Vec<&str>) -> Process {
        Process {
            pid: key.pid,
            start_time_ticks: key.start_time_ticks,
            boot_id: key.boot_id.clone(),
            key,
            user: None,
            exe: None,
            cmdline: cmdline.into_iter().map(str::to_string).collect(),
            cwd: None,
            ppid,
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: None,
            lazyadmin_run_id: None,
            environment: RedactedEnvironmentSummary::default(),
            provenance: vec![],
        }
    }

    fn listener(id: ListenerId, port: u16, owners: Vec<EntityRef>) -> Listener {
        Listener {
            id,
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("127.0.0.1".into()),
            port: Some(port),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::Loopback,
            owners,
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            dual_stack_state: DualStackState::NotApplicable,
        }
    }
}
