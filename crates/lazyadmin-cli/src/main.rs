#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre::eyre;
use futures::StreamExt;
use lazyadmin_core::{
    actions::{
        Action, ActionKind, ActionPlan, ActionResult, ActionStatus, ConfirmationPolicy,
        plan_free_port_for_snapshot,
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
    model::{DIFF_SCHEMA_VERSION, EntityRef, Listener, Process, ProcessKey, Protocol, Snapshot},
    output::listener_rows,
    selector::{Selector, parse_selector},
};
use lazyadmin_runtime::view_model::search::{SearchKinds, SearchOptions, SearchResults};
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
    #[arg(
        short = 'p',
        long = "port",
        value_name = "PORT",
        help = "Inspect what is running on a port and its process tree"
    )]
    port: Option<u16>,
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
    Overview,
    Logs(LogsArgs),
    Doctor(DoctorArgs),
    Events(EventsArgs),
    Export,
    Diff(DiffArgs),
    Search(SearchArgs),
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
#[command(
    after_help = "Views: overview, listeners, workloads, processes, doctor, metrics. Legacy filtered views remain addressable with --view public, --view conflicts, --view orphans, --view everything. In the Listeners view use chips/keys: A all, P public, C conflicts, O orphans, U unowned, T tracked."
)]
struct TuiArgs {
    #[arg(long)]
    headless: bool,
    #[arg(long)]
    theme: Option<String>,
    #[arg(long)]
    view: Option<String>,
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
struct SearchArgs {
    /// The search query (text, port number, or PID)
    query: String,
    /// Filter results to a specific entity kind
    #[arg(long, value_enum)]
    kind: Option<SearchEntityKind>,
    /// Maximum number of results per group
    #[arg(long, value_parser = parse_search_limit)]
    limit: Option<usize>,
}

#[derive(Clone, Debug, ValueEnum)]
enum SearchEntityKind {
    Listeners,
    Processes,
    Workloads,
    Projects,
    Managers,
    All,
}

fn parse_search_limit(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid search limit `{value}`"))?;
    if (1..=lazyadmin_runtime::view_model::search::MAX_SEARCH_LIMIT).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!(
            "search limit must be between 1 and {}",
            lazyadmin_runtime::view_model::search::MAX_SEARCH_LIMIT
        ))
    }
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
    if let Some(port) = cli.port {
        if cli.command.is_some() {
            return Err(AppError::Other(eyre!(
                "--port cannot be combined with a subcommand; use lazyadmin --port {port} or lazyadmin port {port}"
            )));
        }
        return run_port_inspect(port, cli.json, cli.brief, cli.config.as_deref()).await;
    }
    match cli.command {
        None => {
            if let Some(selector) = cli.selector {
                run_point_query(&selector, cli.json, cli.brief, cli.config.as_deref()).await
            } else {
                run_tui_command(
                    TuiArgs {
                        headless: false,
                        theme: None,
                        view: None,
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
            run_port_inspect(port, cli.json, cli.brief, cli.config.as_deref()).await
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
        Some(Command::Overview) => run_overview(cli.json, cli.config.as_deref()).await,
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
        Some(Command::Search(args)) => {
            run_search_command(args, cli.json, cli.config.as_deref()).await
        }
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
    let initial_view = match args.view.as_deref() {
        Some(view) => Some(
            lazyadmin_tui::parse_view_kind(view)
                .ok_or_else(|| AppError::Other(eyre!("unknown TUI view {view}")))?,
        ),
        None => None,
    };
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
    let live_feed = lazyadmin_runtime::spawn_live_snapshot_feed(
        cfg.clone(),
        config_path.map(std::path::Path::to_path_buf),
        lazyadmin_runtime::LiveSnapshotFeedSettings::from_config(&cfg),
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
        snapshots: Some(live_feed.snapshots),
        discovery_events: Some(live_feed.events),
        initial_view,
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

async fn run_search_command(
    args: SearchArgs,
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let span = info_span!(
        "cli.search",
        query_len = args.query.len(),
        kind_filter = ?args.kind,
    );
    let _guard = span.enter();

    let snap = build_snapshot(config_path).await?;

    let kinds = match args.kind {
        None | Some(SearchEntityKind::All) => SearchKinds::default(),
        Some(SearchEntityKind::Listeners) => SearchKinds {
            listeners: true,
            processes: false,
            workloads: false,
            projects: false,
            managers: false,
            rail_views: false,
        },
        Some(SearchEntityKind::Processes) => SearchKinds {
            listeners: false,
            processes: true,
            workloads: false,
            projects: false,
            managers: false,
            rail_views: false,
        },
        Some(SearchEntityKind::Workloads) => SearchKinds {
            listeners: false,
            processes: false,
            workloads: true,
            projects: false,
            managers: false,
            rail_views: false,
        },
        Some(SearchEntityKind::Projects) => SearchKinds {
            listeners: false,
            processes: false,
            workloads: false,
            projects: true,
            managers: false,
            rail_views: false,
        },
        Some(SearchEntityKind::Managers) => SearchKinds {
            listeners: false,
            processes: false,
            workloads: false,
            projects: false,
            managers: true,
            rail_views: false,
        },
    };

    let limit = args
        .limit
        .unwrap_or(lazyadmin_runtime::view_model::search::DEFAULT_SEARCH_LIMIT);

    let options = SearchOptions {
        limit,
        show_system: true,
        kinds,
    };

    let results = lazyadmin_runtime::view_model::search::run(&snap, &args.query, options);

    if json {
        print_json(&results)?;
    } else {
        print_search_human(&results);
    }
    Ok(())
}

fn print_search_human(results: &SearchResults) {
    if !results.strategy_hint.is_empty() {
        println!("strategy: {}", results.strategy_hint);
    }

    let total_hits = results.listeners.total
        + results.processes.total
        + results.workloads.total
        + results.projects.total
        + results.managers.total
        + results.rail_views.total;

    if total_hits == 0 {
        println!("no results");
        return;
    }

    // Listeners
    if !results.listeners.hits.is_empty() {
        println!(
            "\nListeners ({}/{})",
            results.listeners.returned, results.listeners.total
        );
        for hit in &results.listeners.hits {
            let owner = if hit.owner_label.is_empty() {
                String::new()
            } else {
                format!("  owner={}", hit.owner_label)
            };
            println!(
                "  [{:>5}] {:>5} {:6} {:<24} {:?}{}",
                hit.score,
                hit.port.map(|p| p.to_string()).unwrap_or_default(),
                format!("{:?}", hit.protocol).to_lowercase(),
                hit.bind,
                hit.exposure,
                owner,
            );
        }
        if results.listeners.truncated {
            println!(
                "  … +{} more",
                results.listeners.total - results.listeners.returned
            );
        }
    }

    // Processes
    if !results.processes.hits.is_empty() {
        println!(
            "\nProcesses ({}/{})",
            results.processes.returned, results.processes.total
        );
        for hit in &results.processes.hits {
            let user = hit.user.as_deref().unwrap_or("-");
            println!(
                "  [{:>5}] pid={:<6} user={:<12} {}",
                hit.score, hit.pid, user, hit.exe_or_argv0,
            );
        }
        if results.processes.truncated {
            println!(
                "  … +{} more",
                results.processes.total - results.processes.returned
            );
        }
    }

    // Workloads
    if !results.workloads.hits.is_empty() {
        println!(
            "\nWorkloads ({}/{})",
            results.workloads.returned, results.workloads.total
        );
        for hit in &results.workloads.hits {
            println!(
                "  [{:>5}] {:<24} runtime={} listeners={} pids={}",
                hit.score, hit.display_name, hit.runtime, hit.listener_count, hit.pid_count,
            );
        }
        if results.workloads.truncated {
            println!(
                "  … +{} more",
                results.workloads.total - results.workloads.returned
            );
        }
    }

    // Projects
    if !results.projects.hits.is_empty() {
        println!(
            "\nProjects ({}/{})",
            results.projects.returned, results.projects.total
        );
        for hit in &results.projects.hits {
            println!(
                "  [{:>5}] {:<24} {}",
                hit.score,
                hit.name,
                hit.root.display(),
            );
        }
        if results.projects.truncated {
            println!(
                "  … +{} more",
                results.projects.total - results.projects.returned
            );
        }
    }

    // Managers
    if !results.managers.hits.is_empty() {
        println!(
            "\nManagers ({}/{})",
            results.managers.returned, results.managers.total
        );
        for hit in &results.managers.hits {
            let avail = if hit.available { "up" } else { "down" };
            println!(
                "  [{:>5}] {:<24} kind={} scope={} {}",
                hit.score, hit.name, hit.kind, hit.scope, avail,
            );
        }
        if results.managers.truncated {
            println!(
                "  … +{} more",
                results.managers.total - results.managers.returned
            );
        }
    }

    println!(
        "\n({total_hits} total matches, {:.1}ms)",
        results.elapsed_ms
    );
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

async fn run_overview(
    json: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let snapshot = build_snapshot(config_path).await?;
    let digest = lazyadmin_runtime::view_model::build_digest(&snapshot);
    if json {
        print_json(&digest)?;
        return Ok(());
    }
    println!(
        "Overview: exposed {} public / {} LAN, conflicts {}, projects {}, triage {} actionable",
        digest.exposed.total_public,
        digest.exposed.total_lan,
        digest.conflicts.total,
        digest.your_projects.total,
        digest.triage.summary.actionable
    );
    if digest.exposed.rows.is_empty() {
        println!("Exposed: {}", digest.exposed.empty_copy);
    } else {
        println!("Exposed:");
        for row in &digest.exposed.rows {
            let folded = if row.extra_ports > 0 {
                format!(" (+{} ports)", row.extra_ports)
            } else {
                String::new()
            };
            println!(
                "  {} {} owner={}{}",
                row.bind,
                exposure_word(&row.exposure),
                row.owner_label,
                folded
            );
        }
    }
    if digest.conflicts.rows.is_empty() {
        println!("Conflicts: {}", digest.conflicts.empty_copy);
    } else {
        println!("Conflicts:");
        for row in &digest.conflicts.rows {
            println!("  {} owners={} {}", row.bind, row.owner_count, row.reason);
        }
    }
    if digest.your_projects.rows.is_empty() {
        println!("Projects: {}", digest.your_projects.empty_copy);
    } else {
        println!("Projects:");
        for row in &digest.your_projects.rows {
            println!(
                "  {} listeners={} workloads={}",
                row.name, row.listener_count, row.workload_count
            );
        }
    }
    Ok(())
}

fn exposure_word(exposure: &lazyadmin_core::model::Exposure) -> &'static str {
    match exposure {
        lazyadmin_core::model::Exposure::Public => "public",
        lazyadmin_core::model::Exposure::LanOrPublic => "lan",
        lazyadmin_core::model::Exposure::Loopback => "loopback",
        lazyadmin_core::model::Exposure::ContainerOnly => "container",
        lazyadmin_core::model::Exposure::UnixLocal => "unix",
        lazyadmin_core::model::Exposure::Unknown => "unknown",
    }
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

/// Inspect a single port: which listener(s) are bound to it, the owning
/// process(es), and the surrounding process tree (ancestors and descendants).
async fn run_port_inspect(
    port: u16,
    json: bool,
    brief: bool,
    config_path: Option<&std::path::Path>,
) -> std::result::Result<(), AppError> {
    let snap = build_snapshot(config_path).await?;
    let listeners: Vec<&Listener> = snap
        .listeners
        .iter()
        .filter(|l| l.port == Some(port))
        .collect();

    let mut owner_keys: Vec<ProcessKey> = Vec::new();
    for listener in &listeners {
        collect_owner_process_keys(listener, &snap, &mut owner_keys);
    }
    let owner_refs = port_owner_refs(&listeners);
    let owners: Vec<&Process> = owner_keys
        .iter()
        .filter_map(|key| snap.processes.iter().find(|p| &p.key == key))
        .collect();

    if json {
        print_json(&port_report_json(
            port,
            &listeners,
            &owner_refs,
            &owners,
            &snap,
        ))?;
        return Ok(());
    }

    if listeners.is_empty() {
        println!("no listener found on :{port}");
        return Ok(());
    }

    let label = if listeners.len() == 1 {
        "listener"
    } else {
        "listeners"
    };
    println!(":{port} — {} {label}", listeners.len());
    for listener in &listeners {
        println!(
            "  {} {}:{}  [{}]  state={} confidence={}",
            format!("{:?}", listener.protocol).to_lowercase(),
            listener.bind_addr.as_deref().unwrap_or("*"),
            listener.port.unwrap_or(port),
            exposure_word(&listener.exposure),
            format!("{:?}", listener.state).to_lowercase(),
            format!("{:?}", listener.confidence).to_lowercase(),
        );
    }

    if brief {
        println!();
        if owners.is_empty() {
            println!("owner: unknown");
        }
        for owner in &owners {
            println!("owner: {}", format_process(owner));
        }
        return Ok(());
    }

    println!();
    if owners.is_empty() {
        println!("what's running on :{port}: owner could not be resolved");
        print_unresolved_owner_hint(&listeners);
    } else {
        println!("what's running on :{port}");
        for owner in &owners {
            print!("  {}", format_process(owner));
            if let Some(extra) = owner_associations(owner, &snap) {
                print!("  ({extra})");
            }
            println!();
        }
    }

    if !owners.is_empty() && !snap.processes.is_empty() {
        let by_pid = process_by_pid(&snap);
        let children = children_by_ppid(&snap);
        for owner in &owners {
            println!();
            println!("process tree (owner pid {})", owner.pid);
            let mut visited = std::collections::HashSet::new();
            let chain = ancestor_chain(owner, &by_pid);
            for (depth, ancestor) in chain.iter().enumerate() {
                visited.insert(ancestor.pid);
                println!("{}{}", tree_indent(depth), format_process(ancestor));
            }
            print_process_subtree(owner, chain.len(), &owner.key, &children, &mut visited);
        }
    }

    let warnings: Vec<&lazyadmin_core::model::Warning> = snap
        .warnings
        .iter()
        .filter(|w| warning_relevant_to_port(w, &listeners, &owner_refs))
        .collect();
    if !warnings.is_empty() {
        println!();
        println!("warnings");
        for w in warnings {
            println!("  {}: {}", w.code, w.message);
        }
    }

    Ok(())
}

/// Resolve the process keys that own a listener, expanding workload owners to
/// their member pids.
fn collect_owner_process_keys(listener: &Listener, snap: &Snapshot, out: &mut Vec<ProcessKey>) {
    for owner in &listener.owners {
        match owner {
            EntityRef::Process(key) => push_unique_key(out, key.clone()),
            EntityRef::Workload(id) => {
                if let Some(workload) = snap.workloads.iter().find(|w| &w.id == id) {
                    for key in &workload.pids {
                        push_unique_key(out, key.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

fn push_unique_key(out: &mut Vec<ProcessKey>, key: ProcessKey) {
    if !out.contains(&key) {
        out.push(key);
    }
}

fn port_owner_refs(listeners: &[&Listener]) -> Vec<EntityRef> {
    let mut refs = Vec::new();
    for listener in listeners {
        for owner in &listener.owners {
            if !refs.contains(owner) {
                refs.push(owner.clone());
            }
        }
    }
    refs
}

fn process_by_pid(snap: &Snapshot) -> std::collections::HashMap<i32, &Process> {
    let mut map = std::collections::HashMap::new();
    for p in &snap.processes {
        map.entry(p.pid).or_insert(p);
    }
    map
}

fn children_by_ppid(snap: &Snapshot) -> std::collections::HashMap<i32, Vec<&Process>> {
    let mut map: std::collections::HashMap<i32, Vec<&Process>> = std::collections::HashMap::new();
    for p in &snap.processes {
        if let Some(ppid) = p.ppid {
            map.entry(ppid).or_default().push(p);
        }
    }
    for kids in map.values_mut() {
        kids.sort_by_key(|p| (p.pid, p.start_time_ticks));
    }
    map
}

/// Walk from a process up to its root via ppid, returning ancestors root-first.
fn ancestor_chain<'a>(
    owner: &'a Process,
    by_pid: &std::collections::HashMap<i32, &'a Process>,
) -> Vec<&'a Process> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(owner.pid);
    let mut current = owner;
    while let Some(ppid) = current.ppid {
        let Some(parent) = by_pid.get(&ppid) else {
            break;
        };
        if !seen.insert(parent.pid) {
            break; // cycle guard
        }
        chain.push(*parent);
        current = parent;
        if chain.len() > 64 {
            break;
        }
    }
    chain.reverse();
    chain
}

fn print_process_subtree(
    process: &Process,
    depth: usize,
    owner_key: &ProcessKey,
    children: &std::collections::HashMap<i32, Vec<&Process>>,
    visited: &mut std::collections::HashSet<i32>,
) {
    if !visited.insert(process.pid) {
        return;
    }
    let marker = if &process.key == owner_key {
        "   ◀ owns this port"
    } else {
        ""
    };
    println!(
        "{}{}{}",
        tree_indent(depth),
        format_process(process),
        marker
    );
    if let Some(kids) = children.get(&process.pid) {
        for child in kids {
            print_process_subtree(child, depth + 1, owner_key, children, visited);
        }
    }
}

fn tree_indent(depth: usize) -> String {
    if depth == 0 {
        String::from("  ")
    } else {
        format!("  {}└─ ", "   ".repeat(depth - 1))
    }
}

fn format_process(p: &Process) -> String {
    let command = if p.cmdline.is_empty() {
        p.exe
            .as_ref()
            .map(|e| e.display().to_string())
            .unwrap_or_else(|| "<unknown>".into())
    } else {
        p.cmdline.join(" ")
    };
    let command = truncate_text(&command, 80);
    let user = p
        .user
        .as_deref()
        .map(|u| format!(" uid={u}"))
        .unwrap_or_default();
    let runtime = p
        .systemd_unit
        .as_deref()
        .map(|unit| format!(" [{unit}]"))
        .or_else(|| {
            p.container_id
                .as_deref()
                .map(|id| format!(" [container {}]", &id[..id.len().min(12)]))
        })
        .unwrap_or_default();
    format!("pid {}{} {}{}", p.pid, user, command, runtime)
}

/// Workload / project / run associations for an owning process, if any.
fn owner_associations(process: &Process, snap: &Snapshot) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(workload) = snap
        .workloads
        .iter()
        .find(|w| w.pids.contains(&process.key))
    {
        parts.push(format!("workload: {}", workload.display_name));
        if let Some(project_id) = &workload.project {
            if let Some(project) = snap.projects.iter().find(|p| &p.id == project_id) {
                parts.push(format!("project: {}", project.name));
            }
        }
    }
    if let Some(run_id) = &process.lazyadmin_run_id {
        parts.push(format!("run: {run_id}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn print_unresolved_owner_hint(listeners: &[&Listener]) {
    let any_low = listeners
        .iter()
        .any(|l| matches!(l.confidence, lazyadmin_core::model::Confidence::Low));
    if any_low {
        println!("  (low confidence — try running with elevated privileges for owner attribution)");
    }
}

fn warning_relevant_to_port(
    warning: &lazyadmin_core::model::Warning,
    listeners: &[&Listener],
    owner_refs: &[EntityRef],
) -> bool {
    let Some(entity) = warning.entity.as_ref() else {
        return false;
    };
    match entity {
        EntityRef::Listener(id) => listeners.iter().any(|l| &l.id == id),
        _ => owner_refs.contains(entity),
    }
}

fn truncate_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

fn port_report_json(
    port: u16,
    listeners: &[&Listener],
    owner_refs: &[EntityRef],
    owners: &[&Process],
    snap: &Snapshot,
) -> serde_json::Value {
    let by_pid = process_by_pid(snap);
    let children = children_by_ppid(snap);
    let processes: Vec<serde_json::Value> = owners
        .iter()
        .map(|owner| {
            let ancestors: Vec<i32> = ancestor_chain(owner, &by_pid)
                .iter()
                .map(|p| p.pid)
                .collect();
            let mut descendants = Vec::new();
            collect_descendant_pids(
                owner.pid,
                &children,
                &mut descendants,
                &mut std::collections::HashSet::new(),
            );
            serde_json::json!({
                "process": owner,
                "ancestor_pids": ancestors,
                "descendant_pids": descendants,
                "associations": owner_associations(owner, snap),
            })
        })
        .collect();
    let warnings: Vec<&lazyadmin_core::model::Warning> = snap
        .warnings
        .iter()
        .filter(|w| warning_relevant_to_port(w, listeners, owner_refs))
        .collect();
    serde_json::json!({
        "port": port,
        "listeners": listeners,
        "owners": processes,
        "warnings": warnings,
    })
}

fn collect_descendant_pids(
    pid: i32,
    children: &std::collections::HashMap<i32, Vec<&Process>>,
    out: &mut Vec<i32>,
    visited: &mut std::collections::HashSet<i32>,
) {
    if !visited.insert(pid) {
        return;
    }
    if let Some(kids) = children.get(&pid) {
        for child in kids {
            out.push(child.pid);
            collect_descendant_pids(child.pid, children, out, visited);
        }
    }
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
    let parts = plan_free_port_for_snapshot(&before, args.port, true);
    let mut actions = parts.actions();
    let plan = ActionPlan {
        id: format!("free-{}", args.port),
        created_at: chrono::Utc::now(),
        target: format!(":{}", args.port),
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        dry_run: parts.dry_run(args.port),
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
    let direct_parts = plan_free_port_for_snapshot(&direct_snapshot, args.port, false);
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
        AddressFamily, Confidence, DualStackState, Exposure, Listener, ListenerId, ListenerState,
        Process, ProcessKey, RedactedEnvironmentSummary, Warning, WarningSeverity,
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

    fn workload(id: &str, name: &str, pids: Vec<ProcessKey>) -> lazyadmin_core::model::Workload {
        lazyadmin_core::model::Workload {
            id: lazyadmin_core::model::WorkloadId::new(id),
            display_name: name.into(),
            runtime: lazyadmin_core::model::RuntimeKind::Direct,
            state: lazyadmin_core::model::WorkloadState::Running,
            pids,
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
        }
    }

    fn warning(code: &str, entity: Option<EntityRef>) -> Warning {
        Warning {
            severity: WarningSeverity::Warning,
            code: code.into(),
            message: format!("{code} message"),
            entity,
            provenance: vec![],
        }
    }

    #[test]
    fn collect_owner_process_keys_resolves_process_and_workload_owners() {
        let mut snap = Snapshot::empty();
        let proc_key = process_key(42, 1);
        let workload_pid = process_key(99, 2);
        snap.processes
            .push(process(proc_key.clone(), None, vec!["node"]));
        snap.processes
            .push(process(workload_pid.clone(), None, vec!["python"]));
        snap.workloads
            .push(workload("wl-1", "api", vec![workload_pid.clone()]));
        let listener = listener(
            ListenerId::new("tcp:127.0.0.1:8000:1"),
            8000,
            vec![
                EntityRef::Process(proc_key.clone()),
                EntityRef::Workload(lazyadmin_core::model::WorkloadId::new("wl-1")),
                EntityRef::Process(proc_key.clone()), // duplicate is deduped
            ],
        );

        let mut keys = Vec::new();
        collect_owner_process_keys(&listener, &snap, &mut keys);
        assert_eq!(keys, vec![proc_key, workload_pid]);
    }

    #[test]
    fn ancestor_chain_walks_root_first_and_breaks_cycles() {
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(process_key(1, 1), None, vec!["init"]));
        snap.processes
            .push(process(process_key(10, 1), Some(1), vec!["bash"]));
        snap.processes
            .push(process(process_key(20, 1), Some(10), vec!["node"]));
        let by_pid = process_by_pid(&snap);
        let owner = snap.processes.iter().find(|p| p.pid == 20).unwrap();
        let chain: Vec<i32> = ancestor_chain(owner, &by_pid)
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(chain, vec![1, 10]); // root-first, owner excluded

        // a self-referential ppid must not loop forever
        let mut cyclic = Snapshot::empty();
        cyclic
            .processes
            .push(process(process_key(7, 1), Some(7), vec!["weird"]));
        let by_pid = process_by_pid(&cyclic);
        let owner = cyclic.processes.iter().find(|p| p.pid == 7).unwrap();
        assert!(ancestor_chain(owner, &by_pid).is_empty());
    }

    #[test]
    fn port_report_json_includes_listener_owner_and_tree() {
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(process_key(1, 1), None, vec!["init"]));
        snap.processes
            .push(process(process_key(10, 1), Some(1), vec!["bash"]));
        let owner_key = process_key(20, 1);
        snap.processes.push(process(
            owner_key.clone(),
            Some(10),
            vec!["node", "server.js"],
        ));
        snap.processes.push(process(
            process_key(30, 1),
            Some(20),
            vec!["node", "worker.js"],
        ));
        snap.listeners.push(listener(
            ListenerId::new("tcp:127.0.0.1:8000:1"),
            8000,
            vec![EntityRef::Process(owner_key.clone())],
        ));

        let listeners: Vec<&Listener> = snap.listeners.iter().collect();
        let owner_refs = port_owner_refs(&listeners);
        let owners: Vec<&Process> = vec![snap.processes.iter().find(|p| p.pid == 20).unwrap()];
        let report = port_report_json(8000, &listeners, &owner_refs, &owners, &snap);
        assert_eq!(report["port"], 8000);
        assert_eq!(report["listeners"].as_array().unwrap().len(), 1);
        let owner = &report["owners"][0];
        assert_eq!(owner["process"]["pid"], 20);
        assert_eq!(owner["ancestor_pids"], serde_json::json!([1, 10]));
        assert_eq!(owner["descendant_pids"], serde_json::json!([30]));
    }

    #[test]
    fn port_report_includes_workload_and_unresolved_process_warnings() {
        let mut snap = Snapshot::empty();
        let workload_id = lazyadmin_core::model::WorkloadId::new("wl-1");
        let resolved_key = process_key(20, 1);
        let unresolved_key = process_key(30, 1);
        snap.processes
            .push(process(resolved_key.clone(), None, vec!["node"]));
        snap.workloads
            .push(workload("wl-1", "api", vec![resolved_key.clone()]));
        snap.listeners.push(listener(
            ListenerId::new("tcp:127.0.0.1:8000:1"),
            8000,
            vec![
                EntityRef::Workload(workload_id.clone()),
                EntityRef::Process(unresolved_key.clone()),
            ],
        ));
        snap.warnings.push(warning(
            "workload.warning",
            Some(EntityRef::Workload(workload_id)),
        ));
        snap.warnings.push(warning(
            "process.unresolved",
            Some(EntityRef::Process(unresolved_key)),
        ));

        let listeners: Vec<&Listener> = snap.listeners.iter().collect();
        let owner_refs = port_owner_refs(&listeners);
        let owners: Vec<&Process> = vec![snap.processes.iter().find(|p| p.pid == 20).unwrap()];
        let report = port_report_json(8000, &listeners, &owner_refs, &owners, &snap);
        let codes: Vec<&str> = report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|warning| warning["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes, vec!["workload.warning", "process.unresolved"]);
    }

    #[test]
    fn format_process_labels_uid_and_truncates_long_commands() {
        let mut p = process(process_key(5, 1), Some(1), vec!["short"]);
        p.user = Some("1000".into());
        assert_eq!(format_process(&p), "pid 5 uid=1000 short");

        let long = "x".repeat(200);
        let mut q = process(process_key(6, 1), None, vec![long.as_str()]);
        q.user = None;
        let rendered = format_process(&q);
        assert!(rendered.ends_with('…'));
        assert!(rendered.chars().count() < 100);
    }

    #[test]
    fn search_empty_query_returns_empty_groups() {
        let snap = Snapshot::empty();
        let results =
            lazyadmin_runtime::view_model::search::run(&snap, "", SearchOptions::default());
        assert_eq!(results.schema_version, "lazyadmin.search.v1");
        assert_eq!(
            results.query.kind,
            lazyadmin_runtime::view_model::search::SearchKind::Empty
        );
        assert_eq!(results.listeners.total, 0);
        assert_eq!(results.processes.total, 0);
        assert!(results.strategy_hint.is_empty());
    }

    #[test]
    fn search_port_query_finds_listener() {
        let mut snap = Snapshot::empty();
        let pk = process_key(42, 1);
        snap.processes.push(process(pk.clone(), None, vec!["node"]));
        snap.listeners.push(listener(
            ListenerId::new("tcp:127.0.0.1:5432:1"),
            5432,
            vec![EntityRef::Process(pk)],
        ));

        let results =
            lazyadmin_runtime::view_model::search::run(&snap, "5432", SearchOptions::default());
        assert_eq!(results.listeners.total, 1);
        assert_eq!(results.listeners.hits[0].port, Some(5432));
        assert_eq!(results.strategy_hint, "port :5432");
        assert!(!results.fell_back_to_prefix);
    }

    #[test]
    fn search_text_query_json_roundtrip() {
        let snap = Snapshot::empty();
        let results =
            lazyadmin_runtime::view_model::search::run(&snap, "hermes", SearchOptions::default());
        let json = serde_json::to_value(&results).unwrap();
        assert_eq!(json["schema_version"], "lazyadmin.search.v1");
        assert_eq!(json["query"]["kind"]["type"], "text");
        assert_eq!(json["strategy_hint"], "text query");
    }

    #[test]
    fn search_kind_filter_restricts_groups() {
        let mut snap = Snapshot::empty();
        let pk = process_key(100, 10);
        snap.processes
            .push(process(pk.clone(), None, vec!["hermes"]));
        snap.listeners.push(listener(
            ListenerId::new("tcp:127.0.0.1:7777:1"),
            7777,
            vec![EntityRef::Process(pk)],
        ));

        // Search with only listeners enabled
        let results = lazyadmin_runtime::view_model::search::run(
            &snap,
            "hermes",
            SearchOptions {
                kinds: SearchKinds {
                    listeners: true,
                    processes: false,
                    workloads: false,
                    projects: false,
                    managers: false,
                    rail_views: false,
                },
                ..Default::default()
            },
        );
        // Processes group should be empty since we disabled it
        assert_eq!(results.processes.total, 0);
    }

    #[test]
    fn search_limit_parser_rejects_out_of_range_values() {
        assert_eq!(parse_search_limit("1").unwrap(), 1);
        assert_eq!(
            parse_search_limit(
                &lazyadmin_runtime::view_model::search::MAX_SEARCH_LIMIT.to_string()
            )
            .unwrap(),
            lazyadmin_runtime::view_model::search::MAX_SEARCH_LIMIT
        );
        assert!(parse_search_limit("0").is_err());
        assert!(parse_search_limit("9999").is_err());
        assert!(parse_search_limit("not-a-number").is_err());
    }
}
