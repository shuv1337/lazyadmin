#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    io, panic,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use color_eyre::eyre::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{
        self, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use lazyadmin_core::{
    config::keybindings::{KeybindAction, ResolvedKeybindings},
    doctor::{WarningTier, metric_caption},
    model::{DiscoveryEvent, EntityRef, Exposure, ProcessKey, Snapshot, WarningSeverity},
    output::listener_rows,
    snapshot::build_empty_snapshot,
};
use lazyadmin_runtime::view_model::{
    Digest, HeaderPip, InspectorView, RAIL_ENTRIES, build_digest, build_doctor_groups,
    inspector::{InspectorRow, InspectorSection as RuntimeInspectorSection, JumpTarget},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, BorderType, Borders, Cell, Clear, List, ListItem,
        Paragraph, Row, Sparkline, Table, TableState, Wrap,
    },
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tracing::{debug, info, info_span};

const NAV_PANE_WIDTH: u16 = 26;
const MAIN_PANE_MIN_WIDTH: u16 = 40;
const INSPECTOR_PANE_WIDTH: u16 = 48;
const THREE_PANE_MIN_WIDTH: u16 = NAV_PANE_WIDTH + MAIN_PANE_MIN_WIDTH + INSPECTOR_PANE_WIDTH;

pub type ConfigReload = Box<dyn FnMut() -> anyhow::Result<(Theme, ResolvedKeybindings)> + Send>;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub refresh_interval: Duration,
    pub show_system: bool,
    pub event_debounce: Duration,
    pub max_redraw_hz: u64,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_millis(500),
            show_system: false,
            event_debounce: Duration::from_millis(100),
            max_redraw_hz: 30,
        }
    }
}

pub struct App {
    pub vm: ViewModel,
    pub snapshot: Snapshot,
    pub pane: Pane,
    pub active_view: ViewKind,
    pub query: String,
    pub mode: InputMode,
    pub should_quit: bool,
    pub show_system: bool,
    pub confirmation: Option<Confirmation>,
    pub theme: Theme,
    pub keybindings: ResolvedKeybindings,
    pub status: Option<String>,
    pub toasts: VecDeque<Toast>,
    pub allow_open_non_loopback: bool,
    selected_row: usize,
    selected_process: Option<ProcessKey>,
    collapsed_processes: HashSet<ProcessKey>,
    doctor_toggled_groups: HashSet<String>,
    doctor_severity_filter: DoctorSeverityFilter,
    event_ring: AdapterEventRing,
    config_reload: Option<ConfigReload>,
    overview_hint_visible: bool,
    listener_filter: ListenerFilter,
    listeners_hint_visible: bool,
    listeners_hint_seen: bool,
    related_listener_filter: Option<RelatedListenerFilter>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RelatedListenerFilter {
    ids: HashSet<String>,
    label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusChannel {
    HeaderPip,
    Toast { ttl: Duration },
    ModalHint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub message: String,
    pub ttl: Duration,
    pub created_at: Option<Instant>,
}

impl App {
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.push_status(
            StatusChannel::Toast {
                ttl: Duration::from_secs(2),
            },
            message,
        );
    }

    pub fn push_status(&mut self, channel: StatusChannel, message: impl Into<String>) {
        let message = message.into();
        match channel {
            StatusChannel::Toast { ttl } => {
                self.status = Some(message.clone());
                self.toasts.push_back(Toast {
                    message,
                    ttl,
                    created_at: Some(Instant::now()),
                });
            }
            StatusChannel::HeaderPip | StatusChannel::ModalHint => {
                self.status = Some(message);
            }
        }
    }
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("vm", &self.vm)
            .field("pane", &self.pane)
            .field("active_view", &self.active_view)
            .field("query", &self.query)
            .field("mode", &self.mode)
            .field("should_quit", &self.should_quit)
            .field("show_system", &self.show_system)
            .field("status", &self.status)
            .field("allow_open_non_loopback", &self.allow_open_non_loopback)
            .finish_non_exhaustive()
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            vm: ViewModel::default(),
            snapshot: build_empty_snapshot(),
            pane: Pane::Rows,
            active_view: ViewKind::Overview,
            query: String::new(),
            mode: InputMode::default(),
            should_quit: false,
            show_system: false,
            confirmation: None,
            theme: Theme::default_dark(),
            keybindings: ResolvedKeybindings {
                bindings: ResolvedKeybindings::default_map()
                    .into_iter()
                    .map(|(a, b)| (a.as_name().into(), b))
                    .collect(),
            },
            status: None,
            toasts: VecDeque::new(),
            allow_open_non_loopback: false,
            selected_row: 0,
            selected_process: None,
            collapsed_processes: HashSet::new(),
            doctor_toggled_groups: HashSet::new(),
            doctor_severity_filter: DoctorSeverityFilter::All,
            event_ring: AdapterEventRing::default(),
            config_reload: None,
            overview_hint_visible: false,
            listener_filter: ListenerFilter::All,
            listeners_hint_visible: false,
            listeners_hint_seen: false,
            related_listener_filter: None,
        }
    }
}

pub struct TuiRuntime {
    pub initial_snapshot: Snapshot,
    pub config: AppConfig,
    pub theme: Theme,
    pub keybindings: ResolvedKeybindings,
    pub color_hint: Option<String>,
    pub allow_open_non_loopback: bool,
    pub snapshots: Option<mpsc::Receiver<Snapshot>>,
    pub discovery_events: Option<mpsc::Receiver<DiscoveryEvent>>,
    pub config_reload: Option<ConfigReload>,
    pub initial_view: Option<ViewKind>,
}

impl TuiRuntime {
    pub fn static_snapshot(snapshot: Snapshot) -> Self {
        Self {
            initial_snapshot: snapshot,
            config: AppConfig::default(),
            theme: Theme::default_dark(),
            keybindings: ResolvedKeybindings {
                bindings: ResolvedKeybindings::default_map()
                    .into_iter()
                    .map(|(a, b)| (a.as_name().into(), b))
                    .collect(),
            },
            color_hint: None,
            allow_open_non_loopback: false,
            snapshots: None,
            discovery_events: None,
            config_reload: None,
            initial_view: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewKind {
    #[default]
    Overview,
    Listeners,
    Workloads,
    Processes,
    Everything,
    Ports,
    Public,
    Conflicts,
    Projects,
    Managers,
    Orphans,
    TrackedRuns,
    Logs,
    Doctor,
    ProcessTree,
    Metrics,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListenerFilter {
    #[default]
    All,
    Public,
    Conflicts,
    Orphans,
    Unowned,
    Tracked,
}

impl ListenerFilter {
    fn label(self) -> &'static str {
        match self {
            ListenerFilter::All => "All",
            ListenerFilter::Public => "Public",
            ListenerFilter::Conflicts => "Conflicts",
            ListenerFilter::Orphans => "Orphans",
            ListenerFilter::Unowned => "Unowned",
            ListenerFilter::Tracked => "Tracked",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    #[default]
    Groups,
    Rows,
    Inspector,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Normal,
    Filter,
    Palette,
    Help,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    pub command: Command,
    pub typed: String,
    pub required: String,
    pub target: String,
    pub command_preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Filter,
    Palette,
    NextPane,
    PrevPane,
    Inspect,
    Logs,
    Ports,
    Tree,
    Metrics,
    Restart,
    Stop,
    Free,
    Kill,
    Open,
    Edit,
    CopyDiagnostic,
    ToggleSystem,
    Run,
    Help,
    Quit,
    Refresh,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub base_fg: ColorSpec,
    pub base_bg: ColorSpec,
    pub accent: ColorSpec,
    pub ok: ColorSpec,
    pub info: ColorSpec,
    pub warning: ColorSpec,
    pub degraded: ColorSpec,
    pub error: ColorSpec,
    pub selection: ColorSpec,
    pub footer: ColorSpec,
    pub risk_public: ColorSpec,
    pub risk_lan: ColorSpec,
    pub risk_loopback: ColorSpec,
    pub marker_conflict: ColorSpec,
    pub marker_tracked: ColorSpec,
    pub marker_project: ColorSpec,
    pub system_noise: ColorSpec,
    pub pip_ok: ColorSpec,
    pub pip_warn: ColorSpec,
    pub pip_error: ColorSpec,
    pub fallback_palette: PaletteMode,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    base_fg: Option<ColorSpec>,
    base_bg: Option<ColorSpec>,
    accent: Option<ColorSpec>,
    ok: Option<ColorSpec>,
    info: Option<ColorSpec>,
    warning: Option<ColorSpec>,
    degraded: Option<ColorSpec>,
    error: Option<ColorSpec>,
    selection: Option<ColorSpec>,
    footer: Option<ColorSpec>,
    risk_public: Option<ColorSpec>,
    risk_lan: Option<ColorSpec>,
    risk_loopback: Option<ColorSpec>,
    marker_conflict: Option<ColorSpec>,
    marker_tracked: Option<ColorSpec>,
    marker_project: Option<ColorSpec>,
    system_noise: Option<ColorSpec>,
    pip_ok: Option<ColorSpec>,
    pip_warn: Option<ColorSpec>,
    pip_error: Option<ColorSpec>,
    fallback_palette: Option<PaletteMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteMode {
    Monochrome,
    Sixteen,
    TwoFiftySix,
    Truecolor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ColorSpec(pub String);

impl ColorSpec {
    pub fn parse(value: &str) -> Result<Self, String> {
        let lower = value.to_ascii_lowercase();
        let named = [
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "gray",
            "white",
            "bright-blue",
        ];
        if named.contains(&lower.as_str())
            || (lower.starts_with('#')
                && (lower.len() == 7 || lower.len() == 9)
                && lower[1..].chars().all(|c| c.is_ascii_hexdigit()))
        {
            Ok(Self(value.into()))
        } else {
            Err(format!("invalid color string `{value}`"))
        }
    }
    fn color(&self) -> Color {
        match self.0.to_ascii_lowercase().as_str() {
            "black" => Color::Black,
            "red" => Color::Red,
            "green" => Color::Green,
            "yellow" => Color::Yellow,
            "blue" => Color::Blue,
            "magenta" => Color::Magenta,
            "cyan" => Color::Cyan,
            "gray" => Color::Gray,
            "white" => Color::White,
            "bright-blue" => Color::LightBlue,
            s if s.starts_with('#') && s.len() >= 7 => {
                let r = u8::from_str_radix(&s[1..3], 16).unwrap_or(255);
                let g = u8::from_str_radix(&s[3..5], 16).unwrap_or(255);
                let b = u8::from_str_radix(&s[5..7], 16).unwrap_or(255);
                Color::Rgb(r, g, b)
            }
            _ => Color::White,
        }
    }
}

impl Theme {
    pub fn default_dark() -> Self {
        Self::builtin("default-dark").unwrap()
    }
    pub fn builtin(name: &str) -> Option<Self> {
        // Canonical Night Owl palette (Sarah Drasner). The default dark theme
        // is Night Owl; `night-owl` is an explicit alias. Light/high-contrast/
        // solarized variants override individual surfaces below.
        let mut t = Self {
            name: name.into(),
            base_fg: ColorSpec("#d6deeb".into()),
            base_bg: ColorSpec("#011627".into()),
            accent: ColorSpec("#ecc48d".into()),
            ok: ColorSpec("#addb67".into()),
            info: ColorSpec("#82aaff".into()),
            warning: ColorSpec("#f78c6c".into()),
            degraded: ColorSpec("#c792ea".into()),
            error: ColorSpec("#ef5350".into()),
            selection: ColorSpec("#1d3b53".into()),
            footer: ColorSpec("#637777".into()),
            risk_public: ColorSpec("#ef5350".into()),
            risk_lan: ColorSpec("#f78c6c".into()),
            risk_loopback: ColorSpec("#addb67".into()),
            marker_conflict: ColorSpec("#ef5350".into()),
            marker_tracked: ColorSpec("#82aaff".into()),
            marker_project: ColorSpec("#ecc48d".into()),
            system_noise: ColorSpec("#637777".into()),
            pip_ok: ColorSpec("#addb67".into()),
            pip_warn: ColorSpec("#f78c6c".into()),
            pip_error: ColorSpec("#ef5350".into()),
            fallback_palette: PaletteMode::Truecolor,
        };
        match name {
            "default-dark" | "night-owl" => {
                t.name = name.into();
                Some(t)
            }
            "default-light" | "night-owl-light" => {
                // Night Owl light by Sarah Drasner.
                t.name = name.into();
                t.base_fg = ColorSpec("#403f53".into());
                t.base_bg = ColorSpec("#fbfbfb".into());
                t.accent = ColorSpec("#daaa01".into());
                t.ok = ColorSpec("#2aa298".into());
                t.info = ColorSpec("#288ed7".into());
                t.warning = ColorSpec("#bc5454".into());
                t.degraded = ColorSpec("#994cc3".into());
                t.error = ColorSpec("#de3d3b".into());
                t.selection = ColorSpec("#d3e8f8".into());
                t.footer = ColorSpec("#90a7b2".into());
                t.risk_public = ColorSpec("#de3d3b".into());
                t.risk_lan = ColorSpec("#bc5454".into());
                t.risk_loopback = ColorSpec("#2aa298".into());
                t.marker_conflict = ColorSpec("#de3d3b".into());
                t.marker_tracked = ColorSpec("#288ed7".into());
                t.marker_project = ColorSpec("#daaa01".into());
                t.system_noise = ColorSpec("#90a7b2".into());
                t.pip_ok = ColorSpec("#2aa298".into());
                t.pip_warn = ColorSpec("#bc5454".into());
                t.pip_error = ColorSpec("#de3d3b".into());
                Some(t)
            }
            "high-contrast" => {
                t.base_fg = ColorSpec("white".into());
                t.base_bg = ColorSpec("black".into());
                t.accent = ColorSpec("yellow".into());
                t.info = ColorSpec("cyan".into());
                t.selection = ColorSpec("blue".into());
                t.footer = ColorSpec("white".into());
                t.fallback_palette = PaletteMode::Sixteen;
                Some(t)
            }
            "colorblind-safe" => {
                t.name = name.into();
                t.risk_public = ColorSpec("#d55e00".into());
                t.risk_lan = ColorSpec("#e69f00".into());
                t.risk_loopback = ColorSpec("#009e73".into());
                t.marker_conflict = ColorSpec("#d55e00".into());
                t.marker_tracked = ColorSpec("#0072b2".into());
                t.marker_project = ColorSpec("#cc79a7".into());
                t.pip_ok = ColorSpec("#009e73".into());
                t.pip_warn = ColorSpec("#e69f00".into());
                t.pip_error = ColorSpec("#d55e00".into());
                Some(t)
            }
            "solarized-dark" => {
                t.base_bg = ColorSpec("#002b36".into());
                t.base_fg = ColorSpec("#839496".into());
                t.accent = ColorSpec("#268bd2".into());
                t.selection = ColorSpec("#073642".into());
                t.footer = ColorSpec("#586e75".into());
                Some(t)
            }
            _ => None,
        }
    }
    pub fn load(name: Option<&str>, path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        tracing::info!("tui.theme.load");
        if let Some(path) = path {
            return Self::load_file(path, None);
        }
        let name = name.unwrap_or("default-dark");
        if let Some(theme) = Self::builtin(name) {
            return Ok(theme);
        }
        if let Some(path) = xdg_theme_path(name) {
            return Self::load_file(&path, Some(name));
        }
        Err(anyhow::anyhow!("unknown theme `{name}`"))
    }
    fn load_file(path: &Path, override_name: Option<&str>) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let file: ThemeFile = toml::from_str(&text)?;
        let mut theme = Theme::default_dark();
        theme.name = override_name
            .or(file.name.as_deref())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("custom")
            })
            .to_string();
        if let Some(value) = file.base_fg {
            theme.base_fg = value;
        }
        if let Some(value) = file.base_bg {
            theme.base_bg = value;
        }
        if let Some(value) = file.accent {
            theme.accent = value;
        }
        if let Some(value) = file.ok {
            theme.ok = value;
        }
        if let Some(value) = file.info {
            theme.info = value;
        }
        if let Some(value) = file.warning {
            theme.warning = value;
        }
        if let Some(value) = file.degraded {
            theme.degraded = value;
        }
        if let Some(value) = file.error {
            theme.error = value;
        }
        if let Some(value) = file.selection {
            theme.selection = value;
        }
        if let Some(value) = file.footer {
            theme.footer = value;
        }
        if let Some(value) = file.risk_public {
            theme.risk_public = value;
        }
        if let Some(value) = file.risk_lan {
            theme.risk_lan = value;
        }
        if let Some(value) = file.risk_loopback {
            theme.risk_loopback = value;
        }
        if let Some(value) = file.marker_conflict {
            theme.marker_conflict = value;
        }
        if let Some(value) = file.marker_tracked {
            theme.marker_tracked = value;
        }
        if let Some(value) = file.marker_project {
            theme.marker_project = value;
        }
        if let Some(value) = file.system_noise {
            theme.system_noise = value;
        }
        if let Some(value) = file.pip_ok {
            theme.pip_ok = value;
        }
        if let Some(value) = file.pip_warn {
            theme.pip_warn = value;
        }
        if let Some(value) = file.pip_error {
            theme.pip_error = value;
        }
        if let Some(value) = file.fallback_palette {
            theme.fallback_palette = value;
        }
        theme.validate()?;
        Ok(theme)
    }
    pub fn validate(&mut self) -> anyhow::Result<()> {
        for c in [
            &self.base_fg,
            &self.base_bg,
            &self.accent,
            &self.ok,
            &self.info,
            &self.warning,
            &self.degraded,
            &self.error,
            &self.selection,
            &self.footer,
            &self.risk_public,
            &self.risk_lan,
            &self.risk_loopback,
            &self.marker_conflict,
            &self.marker_tracked,
            &self.marker_project,
            &self.system_noise,
            &self.pip_ok,
            &self.pip_warn,
            &self.pip_error,
        ] {
            ColorSpec::parse(&c.0).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub fn downgrade_for_colors(mut self, colors: u16) -> (Self, Option<String>) {
        if colors <= 16 && self.fallback_palette != PaletteMode::Sixteen {
            self.fallback_palette = PaletteMode::Sixteen;
            return (self, Some("limited color terminal — using 16".into()));
        }
        (self, None)
    }
}

fn xdg_theme_path(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let candidate = base.join("lazyadmin/themes").join(format!("{name}.toml"));
    candidate.exists().then_some(candidate)
}

pub fn detected_color_count() -> u16 {
    if std::env::var_os("NO_COLOR").is_some() {
        return 16;
    }
    if std::env::var("COLORTERM")
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("truecolor") || v.contains("24bit")
        })
        .unwrap_or(false)
    {
        return u16::MAX;
    }
    if std::env::var("TERM")
        .map(|v| v.contains("256color"))
        .unwrap_or(false)
    {
        return 256;
    }
    16
}

pub struct EventLoop {
    pub rx: mpsc::Receiver<UiEvent>,
}
#[derive(Debug)]
pub enum UiEvent {
    Input(KeyEvent),
    Snapshot(Box<Snapshot>),
    Tick,
    Discovery(DiscoveryEvent),
}

pub struct SnapshotController {
    tx: watch::Sender<Snapshot>,
    handle: JoinHandle<()>,
}

#[derive(Clone, Debug)]
pub struct LiveRefreshState {
    debounce: Duration,
    min_redraw_interval: Duration,
    last_event: Option<Instant>,
    last_redraw: Option<Instant>,
    pending: bool,
    pub events_dropped: u64,
    pub degraded: Option<String>,
    pub coalesced: u64,
}

impl LiveRefreshState {
    pub fn new(debounce: Duration, max_redraw_hz: u64) -> Self {
        Self {
            debounce,
            min_redraw_interval: Duration::from_secs_f64(1.0 / max_redraw_hz.max(1) as f64),
            last_event: None,
            last_redraw: None,
            pending: false,
            events_dropped: 0,
            degraded: None,
            coalesced: 0,
        }
    }
    pub fn on_event(&mut self, event: &DiscoveryEvent, now: Instant) {
        tracing::debug!("tui.event.received");
        self.last_event = Some(now);
        self.pending = true;
        if let Some(adapter) = &event.adapter {
            if matches!(
                event.kind,
                lazyadmin_core::model::DiscoveryEventKind::Degraded
            ) {
                self.degraded = Some(format!(
                    "{}: {}",
                    adapter,
                    event.reason.clone().unwrap_or_default()
                ));
            }
        }
    }
    pub fn set_dropped(&mut self, dropped: u64) {
        self.events_dropped = dropped;
    }
    pub fn should_refresh(&mut self, now: Instant) -> bool {
        if !self.pending {
            return false;
        }
        if self
            .last_event
            .is_some_and(|t| now.duration_since(t) < self.debounce)
        {
            return false;
        }
        if self
            .last_redraw
            .is_some_and(|t| now.duration_since(t) < self.min_redraw_interval)
        {
            self.coalesced += 1;
            tracing::debug!("tui.refresh.coalesced");
            return false;
        }
        self.pending = false;
        self.last_redraw = Some(now);
        true
    }
}
impl SnapshotController {
    pub fn start(config: AppConfig) -> Self {
        let (tx, _) = watch::channel(build_empty_snapshot());
        let tx2 = tx.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.refresh_interval);
            loop {
                interval.tick().await;
                let started = Instant::now();
                let span = info_span!("tui.refresh");
                let _g = span.enter();
                let snap = tokio::task::spawn_blocking(build_empty_snapshot)
                    .await
                    .unwrap_or_else(|_| build_empty_snapshot());
                debug!(
                    elapsed_ms = started.elapsed().as_millis(),
                    "snapshot refreshed"
                );
                if tx2.send(snap).is_err() {
                    break;
                }
            }
        });
        Self { tx, handle }
    }
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.tx.subscribe()
    }
}
impl Drop for SnapshotController {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub struct CommandDispatcher;
impl CommandDispatcher {
    pub fn plan(command: &Command, row: Option<&RowVm>) -> String {
        Self::plan_for_target(command, &action_target(row))
    }

    pub fn plan_for_target(command: &Command, target: &str) -> String {
        format!(
            "Dry run: {command:?} would target {} via lazyadmin-core action planning/execution services",
            target
        )
    }

    pub fn execute(command: &Command) {
        info!(?command, "tui command dispatch requested");
    }
}

pub struct TerminalGuard;
impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub fn install_panic_guard() {
    let old = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        old(info);
    }));
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewModel {
    pub width: u16,
    pub layout: LayoutMode,
    pub groups: Vec<String>,
    pub rows: Vec<RowVm>,
    pub digest: Digest,
    pub conflicts: Vec<SummaryRowVm>,
    pub orphans: Vec<SummaryRowVm>,
    pub projects: Vec<SummaryRowVm>,
    pub workloads: Vec<SummaryRowVm>,
    pub tracked_runs: Vec<SummaryRowVm>,
    pub doctor: DoctorVm,
    pub process_tree: ProcessTreeVm,
    pub metrics: MetricsVm,
    pub header_pip: HeaderPip,
    pub inspector: InspectorVm,
    pub hidden_system_count: usize,
    pub degraded: Option<String>,
    pub events_dropped: u64,
}

impl Default for ViewModel {
    fn default() -> Self {
        let snapshot = build_empty_snapshot();
        Self {
            width: 0,
            layout: LayoutMode::default(),
            groups: Vec::new(),
            rows: Vec::new(),
            digest: Digest::default(),
            conflicts: Vec::new(),
            orphans: Vec::new(),
            projects: Vec::new(),
            workloads: Vec::new(),
            tracked_runs: Vec::new(),
            doctor: DoctorVm::default(),
            process_tree: ProcessTreeVm::default(),
            metrics: MetricsVm::default(),
            header_pip: HeaderPip::from_snapshot(&snapshot),
            inspector: InspectorVm::default(),
            hidden_system_count: 0,
            degraded: None,
            events_dropped: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutMode {
    ThreePane,
    InspectorTab,
    SinglePane,
    #[default]
    Refuse,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowVm {
    pub id: String,
    pub port: Option<u16>,
    pub bind: String,
    pub owner: String,
    pub runtime: String,
    pub exposure: String,
    pub project: String,
    pub badges: Vec<String>,
    #[serde(default)]
    pub is_conflict: bool,
    #[serde(default)]
    pub is_orphan: bool,
    #[serde(default)]
    pub is_tracked: bool,
    #[serde(default)]
    pub is_project: bool,
    #[serde(default)]
    pub is_system: bool,
    pub search_text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryRowVm {
    pub name: String,
    pub kind: String,
    pub state: String,
    pub details: String,
    #[serde(default)]
    pub is_conflict: bool,
    #[serde(default)]
    pub is_tracked: bool,
    #[serde(default)]
    pub is_project: bool,
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorSeverityFilter {
    #[default]
    All,
    Critical,
    Warning,
    Info,
}

impl DoctorSeverityFilter {
    fn matches(self, severity: &WarningSeverity) -> bool {
        match self {
            DoctorSeverityFilter::All => true,
            DoctorSeverityFilter::Critical => matches!(severity, WarningSeverity::Error),
            DoctorSeverityFilter::Warning => matches!(severity, WarningSeverity::Warning),
            DoctorSeverityFilter::Info => matches!(severity, WarningSeverity::Info),
        }
    }

    fn label(self) -> &'static str {
        match self {
            DoctorSeverityFilter::All => "All",
            DoctorSeverityFilter::Critical => "Critical",
            DoctorSeverityFilter::Warning => "Warning",
            DoctorSeverityFilter::Info => "Info",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorVm {
    pub rows: Vec<DoctorRowVm>,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub actionable_count: usize,
    pub noise_group_count: usize,
    pub noise_total_count: usize,
    pub severity_filter: DoctorSeverityFilter,
}

/// Severity classification for a Doctor row, kept in lock-step with
/// `lazyadmin_core::model::WarningSeverity` via an exhaustive `From` impl.
/// Renderers must match on this enum, not on the string `severity` field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    Error,
    Warning,
    #[default]
    Info,
}

impl From<&lazyadmin_core::model::WarningSeverity> for DoctorSeverity {
    fn from(value: &lazyadmin_core::model::WarningSeverity) -> Self {
        match value {
            lazyadmin_core::model::WarningSeverity::Error => DoctorSeverity::Error,
            lazyadmin_core::model::WarningSeverity::Warning => DoctorSeverity::Warning,
            lazyadmin_core::model::WarningSeverity::Info => DoctorSeverity::Info,
        }
    }
}

impl DoctorSeverity {
    pub fn label(self) -> &'static str {
        match self {
            DoctorSeverity::Error => "Error",
            DoctorSeverity::Warning => "Warning",
            DoctorSeverity::Info => "Info",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorRowVm {
    /// Stringly-typed severity label, retained for the public JSON contract
    /// (e.g. `lazyadmin tui --headless --json`). Always equal to
    /// `severity_kind.label()`.
    pub severity: String,
    /// Strongly-typed severity classification used by the renderer and any
    /// code that branches on severity. Renamed in serde so the legacy field
    /// stays the human-friendly one.
    #[serde(default, rename = "severity_kind")]
    pub severity_kind: DoctorSeverity,
    pub check: String,
    pub entity: String,
    pub details: String,
    pub suggested_action: String,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub expanded: bool,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub is_group: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorVm {
    pub title: String,
    /// Legacy plain-text fallback used by logs/diagnostic surfaces and by
    /// non-entity inspector states (overview / empty selections). Entity
    /// inspectors render `sections` instead.
    pub lines: Vec<String>,
    pub provenance: Vec<String>,
    pub provenance_expanded: bool,
    pub diagnostic_markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<InspectorSectionVm>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jump_targets: Vec<JumpTarget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorSectionVm {
    pub heading: String,
    pub rows: Vec<InspectorRow>,
}

impl From<RuntimeInspectorSection> for InspectorSectionVm {
    fn from(section: RuntimeInspectorSection) -> Self {
        Self {
            heading: section.heading.to_string(),
            rows: section.rows,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTreeVm {
    pub rows: Vec<ProcessTreeRow>,
    pub selected: Option<ProcessKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTreeRow {
    pub key: ProcessKey,
    pub depth: usize,
    pub label: String,
    pub runtime: String,
    pub workload: Option<String>,
    pub warnings: Vec<String>,
    pub expanded: bool,
    #[serde(default)]
    pub is_tracked: bool,
    #[serde(default)]
    pub is_project: bool,
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsVm {
    pub listeners_loopback: usize,
    pub listeners_public: usize,
    pub listeners_conflicts: usize,
    pub listeners_orphans: usize,
    pub workloads_by_runtime: Vec<(String, usize)>,
    pub warnings_by_severity: Vec<(String, usize)>,
    pub tracked_runs: usize,
    pub events_dropped: u64,
    pub event_rate: Vec<u64>,
    pub adapters: Vec<AdapterMetricVm>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterMetricVm {
    pub adapter: String,
    pub latency_ms: Option<u64>,
    pub throughput: u64,
    pub drops: u64,
    pub sparkline: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct AdapterEventRing {
    events: BTreeMap<String, VecDeque<Instant>>,
    drops: BTreeMap<String, u64>,
    capacity: usize,
}

impl AdapterEventRing {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: BTreeMap::new(),
            drops: BTreeMap::new(),
            capacity: capacity.max(1),
        }
    }
    pub fn record(&mut self, event: &DiscoveryEvent, now: Instant) {
        let adapter = event.adapter.as_deref().unwrap_or("unknown").to_string();
        let events = self.events.entry(adapter).or_default();
        events.push_back(now);
        while events.len() > self.capacity.max(32) {
            events.pop_front();
        }
    }
    pub fn set_dropped(&mut self, adapter: impl Into<String>, drops: u64) {
        self.drops.insert(adapter.into(), drops);
    }
    pub fn metrics(&self, now: Instant) -> Vec<AdapterMetricVm> {
        self.events
            .iter()
            .map(|(adapter, events)| {
                let recent = events
                    .iter()
                    .filter(|seen| now.duration_since(**seen) <= Duration::from_secs(60))
                    .count() as u64;
                let sparkline = events
                    .iter()
                    .rev()
                    .take(12)
                    .map(|seen| 60u64.saturating_sub(now.duration_since(*seen).as_secs().min(60)))
                    .collect::<Vec<_>>();
                AdapterMetricVm {
                    adapter: adapter.clone(),
                    latency_ms: None,
                    throughput: recent,
                    drops: self.drops.get(adapter).copied().unwrap_or(0),
                    sparkline,
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadlessTuiDump {
    pub schema_version: String,
    pub layout: HeadlessLayout,
    pub panes: Vec<String>,
    pub theme: HeadlessTheme,
    pub keybindings: ResolvedKeybindings,
    pub view_model: ViewModel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadlessLayout {
    pub width: u16,
    pub mode: LayoutMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeadlessTheme {
    pub name: String,
    pub fallback_palette: PaletteMode,
}

pub fn build_view_model(
    snapshot: &Snapshot,
    width: u16,
    show_system: bool,
    filter: &str,
) -> ViewModel {
    build_view_model_with_state(
        snapshot,
        width,
        show_system,
        filter,
        None,
        &HashSet::new(),
        None,
        &HashSet::new(),
        DoctorSeverityFilter::All,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_view_model_with_state(
    snapshot: &Snapshot,
    width: u16,
    show_system: bool,
    filter: &str,
    selected_process: Option<ProcessKey>,
    collapsed_processes: &HashSet<ProcessKey>,
    adapter_metrics: Option<Vec<AdapterMetricVm>>,
    doctor_toggled_groups: &HashSet<String>,
    doctor_severity_filter: DoctorSeverityFilter,
) -> ViewModel {
    let layout = match width {
        THREE_PANE_MIN_WIDTH..=u16::MAX => LayoutMode::ThreePane,
        80..=113 => LayoutMode::InspectorTab,
        60..=79 => LayoutMode::SinglePane,
        _ => LayoutMode::Refuse,
    };
    let mut rows = Vec::new();
    let mut hidden = 0usize;
    let projected_rows = listener_rows(snapshot);
    let conflict_ids: HashSet<_> = snapshot
        .warnings
        .iter()
        .filter(|w| w.code == "CONFLICT")
        .filter_map(|w| match &w.entity {
            Some(EntityRef::Listener(id)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    for l in &snapshot.listeners {
        let is_system = l
            .provenance
            .iter()
            .any(|p| p.claim.contains("systemd:system"));
        if is_system && !show_system {
            hidden += 1;
            continue;
        }
        let projected = projected_rows.iter().find(|row| row.id == l.id);
        let owner = projected
            .and_then(|row| row.manager_detail.clone())
            .unwrap_or_else(|| listener_owner_label(l, snapshot));
        let runtime = projected
            .and_then(|row| row.manager_label.clone())
            .unwrap_or_else(|| listener_runtime_label(l, snapshot, is_system));
        let exposure = exposure_label(&l.exposure);
        let mut badges = Vec::new();
        if matches!(
            l.exposure,
            lazyadmin_core::model::Exposure::LanOrPublic | lazyadmin_core::model::Exposure::Public
        ) {
            badges.push("PUBLIC".into());
        }
        let is_conflict = conflict_ids.contains(&l.id) || l.owners.len() > 1;
        let is_orphan = l.owners.is_empty();
        let is_tracked = listener_is_tracked(l, snapshot);
        let is_project = listener_is_project_member(l, snapshot);
        if is_conflict {
            badges.push("CONFLICT".into());
        }
        if is_orphan {
            badges.push("ORPHAN".into());
        }
        if is_tracked {
            badges.push("TRACKED".into());
        }
        let bind = l.bind_addr.clone().unwrap_or_else(|| {
            l.path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".into())
        });
        let search_text = format!(
            "{:?} {} {} {} {:?} {}",
            l.port, bind, owner, runtime, l.protocol, exposure
        );
        rows.push(RowVm {
            id: l.id.to_string(),
            port: l.port,
            bind,
            owner,
            runtime,
            exposure,
            project: "-".into(),
            badges,
            is_conflict,
            is_orphan,
            is_tracked,
            is_project,
            is_system,
            search_text,
        });
    }
    if !filter.is_empty() {
        let m = SkimMatcherV2::default();
        rows.retain(|r| m.fuzzy_match(&r.search_text, filter).is_some());
    }
    let inspector = selected_process
        .as_ref()
        .and_then(|key| inspector_for_process(snapshot, key))
        .or_else(|| rows.first().map(|row| inspector_for_row(snapshot, row)))
        .unwrap_or_else(|| {
            plain_inspector("No selection", "No workloads/listeners discovered yet")
        });
    let mut process_tree =
        build_process_tree_with_collapsed(snapshot, selected_process.clone(), collapsed_processes);
    if !filter.is_empty() {
        let m = SkimMatcherV2::default();
        process_tree.rows.retain(|r| {
            let text = format!(
                "{} {} {}",
                r.label,
                r.runtime,
                r.workload.clone().unwrap_or_default()
            );
            m.fuzzy_match(&text, filter).is_some()
        });
    }
    let mut conflicts = build_conflict_rows(snapshot);
    let mut orphans = build_orphan_rows(snapshot);
    let mut projects = build_project_rows(snapshot);
    let mut workloads = build_workload_rows(snapshot);
    let mut tracked_runs = build_tracked_run_rows(snapshot);
    let mut doctor =
        build_doctor_vm_with_state(snapshot, doctor_toggled_groups, doctor_severity_filter);
    if !filter.is_empty() {
        let m = SkimMatcherV2::default();
        conflicts.retain(|r| m.fuzzy_match(&summary_search_text(r), filter).is_some());
        orphans.retain(|r| m.fuzzy_match(&summary_search_text(r), filter).is_some());
        projects.retain(|r| m.fuzzy_match(&summary_search_text(r), filter).is_some());
        workloads.retain(|r| m.fuzzy_match(&summary_search_text(r), filter).is_some());
        tracked_runs.retain(|r| m.fuzzy_match(&summary_search_text(r), filter).is_some());
        doctor
            .rows
            .retain(|r| m.fuzzy_match(&doctor_search_text(r), filter).is_some());
    }
    ViewModel {
        width,
        layout,
        groups: groups(show_system),
        rows,
        digest: build_digest(snapshot),
        conflicts,
        orphans,
        projects,
        workloads,
        tracked_runs,
        doctor,
        process_tree,
        metrics: build_metrics_with_adapters(snapshot, None, adapter_metrics.unwrap_or_default()),
        header_pip: HeaderPip::from_snapshot(snapshot),
        inspector,
        hidden_system_count: hidden,
        degraded: snapshot
            .warnings
            .iter()
            .find(|w| w.code.contains("DEGRADED"))
            .map(|w| w.message.clone()),
        events_dropped: snapshot
            .metadata
            .as_ref()
            .and_then(|m| m.events_dropped)
            .unwrap_or(0),
    }
}

fn summary_search_text(row: &SummaryRowVm) -> String {
    format!("{} {} {} {}", row.name, row.kind, row.state, row.details)
}

fn doctor_search_text(row: &DoctorRowVm) -> String {
    format!(
        "{} {} {} {} {}",
        row.severity, row.check, row.entity, row.details, row.suggested_action
    )
}

fn listener_to_summary_row(
    listener: &lazyadmin_core::model::Listener,
    snapshot: &Snapshot,
    details: String,
) -> SummaryRowVm {
    let bind = listener.bind_addr.clone().unwrap_or_else(|| {
        listener
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into())
    });
    let endpoint = match listener.port {
        Some(port) => format!("{bind}:{port}"),
        None => bind.clone(),
    };
    let owner = listener_owner_label(listener, snapshot);
    SummaryRowVm {
        name: endpoint,
        kind: format!("{:?}", listener.protocol).to_ascii_lowercase(),
        state: owner,
        details,
        is_conflict: listener.owners.len() > 1,
        is_tracked: listener_is_tracked(listener, snapshot),
        is_project: listener_is_project_member(listener, snapshot),
        is_system: listener
            .provenance
            .iter()
            .any(|p| p.claim.contains("systemd:system")),
    }
}

/// Build the Conflicts projection.
///
/// Mirrors `lazyadmin conflicts` (see `lazyadmin-cli/src/main.rs`): a listener is
/// in conflict when a `CONFLICT` warning references it OR when it has >1 owner.
fn build_conflict_rows(snapshot: &Snapshot) -> Vec<SummaryRowVm> {
    use std::collections::HashSet;
    let conflict_ids: HashSet<_> = snapshot
        .warnings
        .iter()
        .filter(|w| w.code == "CONFLICT")
        .filter_map(|w| match &w.entity {
            Some(EntityRef::Listener(id)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    snapshot
        .listeners
        .iter()
        .filter(|listener| conflict_ids.contains(&listener.id) || listener.owners.len() > 1)
        .map(|listener| {
            let details = if listener.owners.len() > 1 {
                format!("{} owners contend for this socket", listener.owners.len())
            } else {
                "listed in CONFLICT warning".into()
            };
            SummaryRowVm {
                is_conflict: true,
                ..listener_to_summary_row(listener, snapshot, details)
            }
        })
        .collect()
}

/// Build the Orphans projection.
///
/// Listeners with no resolved owner (no workload/process/manager backing them).
/// Also includes any `ORPHAN`-coded warnings that point at a listener we couldn't
/// match by ID.
fn build_orphan_rows(snapshot: &Snapshot) -> Vec<SummaryRowVm> {
    use std::collections::HashSet;
    let mut seen: HashSet<lazyadmin_core::model::ListenerId> = HashSet::new();
    let mut rows: Vec<SummaryRowVm> = snapshot
        .listeners
        .iter()
        .filter(|listener| listener.owners.is_empty())
        .map(|listener| {
            seen.insert(listener.id.clone());
            listener_to_summary_row(
                listener,
                snapshot,
                "no owning workload/process discovered".into(),
            )
        })
        .collect();
    for warning in &snapshot.warnings {
        if !warning.code.to_ascii_uppercase().contains("ORPHAN") {
            continue;
        }
        let listener_match = match &warning.entity {
            Some(EntityRef::Listener(id)) => snapshot
                .listeners
                .iter()
                .find(|listener| &listener.id == id),
            _ => None,
        };
        match listener_match {
            Some(listener) if !seen.contains(&listener.id) => {
                seen.insert(listener.id.clone());
                rows.push(listener_to_summary_row(
                    listener,
                    snapshot,
                    warning.message.clone(),
                ));
            }
            _ => {}
        }
    }
    rows
}

fn build_project_rows(snapshot: &Snapshot) -> Vec<SummaryRowVm> {
    snapshot
        .projects
        .iter()
        .map(|project| SummaryRowVm {
            name: project.name.clone(),
            kind: project
                .package_manager
                .clone()
                .unwrap_or_else(|| "project".into()),
            state: format!("{} marker(s)", project.markers.len()),
            details: project.root.display().to_string(),
            is_project: true,
            ..SummaryRowVm::default()
        })
        .collect()
}

fn build_workload_rows(snapshot: &Snapshot) -> Vec<SummaryRowVm> {
    snapshot
        .workloads
        .iter()
        .map(|workload| {
            let project = workload
                .project
                .as_ref()
                .and_then(|id| snapshot.projects.iter().find(|project| &project.id == id))
                .map(|project| project.name.clone());
            SummaryRowVm {
                name: workload.display_name.clone(),
                kind: runtime_kind_label(&workload.runtime),
                state: format!("{} process(es)", workload.pids.len()),
                details: project.clone().unwrap_or_else(|| "no project".into()),
                is_tracked: workload.lazyadmin_run_id.is_some(),
                is_project: project.is_some(),
                ..SummaryRowVm::default()
            }
        })
        .collect()
}

fn build_tracked_run_rows(snapshot: &Snapshot) -> Vec<SummaryRowVm> {
    snapshot
        .tracked_runs
        .iter()
        .map(|run| SummaryRowVm {
            name: run
                .tag
                .clone()
                .unwrap_or_else(|| short_id(&run.id.to_string())),
            kind: "tracked run".into(),
            state: format!("{:?}", run.state),
            details: if run.command.is_empty() {
                "-".into()
            } else {
                run.command.join(" ")
            },
            is_tracked: true,
            ..SummaryRowVm::default()
        })
        .collect()
}

fn build_doctor_vm_with_state(
    snapshot: &Snapshot,
    toggled_groups: &HashSet<String>,
    severity_filter: DoctorSeverityFilter,
) -> DoctorVm {
    let grouped = build_doctor_groups(snapshot);
    let mut vm = DoctorVm {
        actionable_count: grouped.actionable_count,
        noise_group_count: grouped.noise_group_count,
        noise_total_count: grouped.noise_total_count,
        severity_filter,
        ..DoctorVm::default()
    };
    for group in grouped.groups {
        let severity_kind = DoctorSeverity::from(&group.severity);
        match severity_kind {
            DoctorSeverity::Error => vm.error_count += group.count,
            DoctorSeverity::Warning => vm.warning_count += group.count,
            DoctorSeverity::Info => vm.info_count += group.count,
        }
        if !severity_filter.matches(&group.severity) {
            continue;
        }
        let key = doctor_group_key(&group.code, &group.severity);
        let expanded = group.expanded ^ toggled_groups.contains(&key);
        let sample = group
            .sample_entities
            .first()
            .map(|entity| format_entity_ref(entity, snapshot))
            .unwrap_or_else(|| "snapshot".into());
        let tier = warning_tier_label(group.tier).to_string();
        let suggested_action = if !expanded && matches!(group.tier, WarningTier::Noise) {
            format!(
                "{}; collapsed noise, press Enter to expand",
                group.remediation
            )
        } else {
            group.remediation.clone()
        };
        vm.rows.push(DoctorRowVm {
            severity: severity_kind.label().to_string(),
            severity_kind,
            check: group.label.clone(),
            entity: sample,
            details: group.code.clone(),
            suggested_action,
            count: group.count,
            tier: tier.clone(),
            expanded,
            code: group.code.clone(),
            is_group: true,
        });
        if expanded {
            for warning in snapshot
                .warnings
                .iter()
                .filter(|warning| warning.code == group.code && warning.severity == group.severity)
            {
                let entity = warning
                    .entity
                    .as_ref()
                    .map(|entity| format_entity_ref(entity, snapshot))
                    .unwrap_or_else(|| "snapshot".into());
                vm.rows.push(DoctorRowVm {
                    severity: severity_kind.label().to_string(),
                    severity_kind,
                    check: format!("↳ {entity}"),
                    entity,
                    details: warning.message.clone(),
                    suggested_action: group.remediation.clone(),
                    count: 0,
                    tier: tier.clone(),
                    expanded: false,
                    code: group.code.clone(),
                    is_group: false,
                });
            }
        }
    }
    vm
}

fn doctor_group_key(code: &str, severity: &WarningSeverity) -> String {
    format!("{code}:{severity:?}")
}

fn warning_tier_label(tier: WarningTier) -> &'static str {
    match tier {
        WarningTier::Critical => "critical",
        WarningTier::Actionable => "actionable",
        WarningTier::Noise => "noise",
    }
}

/// Render an `EntityRef` as a short, human-readable label by resolving it
/// against the snapshot. Falls back to a stable identifier when the referent
/// can't be located (which can happen across a partial snapshot refresh).
fn format_entity_ref(entity: &EntityRef, snapshot: &Snapshot) -> String {
    let label = match entity {
        EntityRef::Listener(id) => snapshot
            .listeners
            .iter()
            .find(|listener| &listener.id == id)
            .map(|listener| {
                let bind = listener.bind_addr.clone().unwrap_or_else(|| {
                    listener
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "-".into())
                });
                match listener.port {
                    Some(port) => format!("listener {bind}:{port}"),
                    None => format!("listener {bind}"),
                }
            })
            .unwrap_or_else(|| format!("listener {}", short_id(&id.to_string()))),
        EntityRef::Process(key) => snapshot
            .processes
            .iter()
            .find(|process| &process.key == key)
            // process_owner_label already returns "<command> pid <n>", so just
            // use it directly rather than re-prefixing the pid.
            .map(process_owner_label)
            .unwrap_or_else(|| format!("pid {}", key.pid)),
        EntityRef::Workload(id) => snapshot
            .workloads
            .iter()
            .find(|workload| &workload.id == id)
            .map(|workload| format!("workload {}", workload.display_name))
            .unwrap_or_else(|| format!("workload {}", short_id(&id.to_string()))),
        EntityRef::Manager(id) => snapshot
            .managers
            .iter()
            .find(|manager| &manager.id == id)
            .map(|manager| format!("manager {}", manager.name))
            .unwrap_or_else(|| format!("manager {}", short_id(&id.to_string()))),
        EntityRef::Project(id) => snapshot
            .projects
            .iter()
            .find(|project| &project.id == id)
            .map(|project| format!("project {}", project.name))
            .unwrap_or_else(|| format!("project {}", short_id(&id.to_string()))),
        EntityRef::Run(id) => format!("run {}", short_id(&id.to_string())),
        EntityRef::Action(id) => format!("action {}", short_id(&id.to_string())),
    };
    compact_text(&label, 32)
}

pub fn build_process_tree(snapshot: &Snapshot, selected: Option<ProcessKey>) -> ProcessTreeVm {
    build_process_tree_with_collapsed(snapshot, selected, &HashSet::new())
}

pub fn build_process_tree_with_collapsed(
    snapshot: &Snapshot,
    selected: Option<ProcessKey>,
    collapsed: &HashSet<ProcessKey>,
) -> ProcessTreeVm {
    let mut children: std::collections::BTreeMap<Option<i32>, Vec<_>> =
        std::collections::BTreeMap::new();
    for p in &snapshot.processes {
        children.entry(p.ppid).or_default().push(p);
    }
    for xs in children.values_mut() {
        xs.sort_by_key(|p| (p.pid, p.start_time_ticks));
    }
    let mut rows = Vec::new();
    let roots: Vec<_> = snapshot
        .processes
        .iter()
        .filter(|p| p.ppid.is_none() || !snapshot.processes.iter().any(|q| Some(q.pid) == p.ppid))
        .collect();
    let roots = if roots.is_empty() {
        snapshot.processes.iter().collect::<Vec<_>>()
    } else {
        roots
    };
    for root in roots {
        push_process_row(root, 0, &children, snapshot, collapsed, &mut rows);
    }
    ProcessTreeVm { rows, selected }
}

fn push_process_row(
    process: &lazyadmin_core::model::Process,
    depth: usize,
    children: &std::collections::BTreeMap<Option<i32>, Vec<&lazyadmin_core::model::Process>>,
    snapshot: &Snapshot,
    collapsed: &HashSet<ProcessKey>,
    rows: &mut Vec<ProcessTreeRow>,
) {
    let child_count = children.get(&Some(process.pid)).map_or(0, Vec::len);
    let is_expanded = child_count > 0 && !collapsed.contains(&process.key);
    let runtime = process
        .systemd_unit
        .as_ref()
        .map(|_| "systemd")
        .or(process.container_id.as_ref().map(|_| "container"))
        .unwrap_or("direct")
        .to_string();
    let workload_ref = snapshot
        .workloads
        .iter()
        .find(|w| w.pids.contains(&process.key));
    let workload = workload_ref.map(|w| w.display_name.clone()).or_else(|| {
        process
            .lazyadmin_run_id
            .as_ref()
            .map(|r| format!("run:{r}"))
    });
    let warnings = snapshot
        .warnings
        .iter()
        .filter(|warning| {
            matches!(
                warning.entity.as_ref(),
                Some(EntityRef::Process(key)) if key == &process.key
            )
        })
        .map(|warning| warning.code.clone())
        .collect::<Vec<_>>();
    rows.push(ProcessTreeRow {
        key: process.key.clone(),
        depth,
        label: format!(
            "{}pid {} {}",
            if depth == 0 { "" } else { "└── " },
            process.pid,
            process
                .cmdline
                .first()
                .cloned()
                .unwrap_or_else(|| "<unknown>".into())
        ),
        runtime,
        workload,
        warnings,
        expanded: is_expanded,
        is_tracked: process.lazyadmin_run_id.is_some()
            || workload_ref.is_some_and(|w| w.lazyadmin_run_id.is_some()),
        is_project: workload_ref.is_some_and(|w| w.project.is_some()),
        is_system: process.systemd_unit.is_some(),
    });
    if is_expanded {
        if let Some(kids) = children.get(&Some(process.pid)) {
            for child in kids {
                push_process_row(child, depth + 1, children, snapshot, collapsed, rows);
            }
        }
    }
}

pub fn build_metrics(snapshot: &Snapshot, previous: Option<&Snapshot>) -> MetricsVm {
    build_metrics_with_adapters(snapshot, previous, Vec::new())
}

pub fn build_metrics_with_adapters(
    snapshot: &Snapshot,
    previous: Option<&Snapshot>,
    adapters: Vec<AdapterMetricVm>,
) -> MetricsVm {
    let listeners_loopback = snapshot
        .listeners
        .iter()
        .filter(|l| matches!(l.exposure, Exposure::Loopback | Exposure::UnixLocal))
        .count();
    let listeners_public = snapshot.listeners.len().saturating_sub(listeners_loopback);
    let conflict_ids: HashSet<_> = snapshot
        .warnings
        .iter()
        .filter(|w| w.code == "CONFLICT")
        .filter_map(|w| match &w.entity {
            Some(EntityRef::Listener(id)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    let listeners_conflicts = snapshot
        .listeners
        .iter()
        .filter(|listener| conflict_ids.contains(&listener.id) || listener.owners.len() > 1)
        .count();
    let listeners_orphans = snapshot
        .listeners
        .iter()
        .filter(|listener| listener.owners.is_empty())
        .count();
    let mut runtimes = std::collections::BTreeMap::<String, usize>::new();
    for w in &snapshot.workloads {
        *runtimes.entry(format!("{:?}", w.runtime)).or_default() += 1;
    }
    let mut severities = std::collections::BTreeMap::<String, usize>::new();
    for w in &snapshot.warnings {
        *severities.entry(format!("{:?}", w.severity)).or_default() += 1;
    }
    let prev_listeners = previous.map_or(snapshot.listeners.len(), |p| p.listeners.len());
    let rate = snapshot.listeners.len().abs_diff(prev_listeners) as u64;
    let event_rate = if adapters.is_empty() {
        vec![rate]
    } else {
        adapters.iter().map(|adapter| adapter.throughput).collect()
    };
    MetricsVm {
        listeners_loopback,
        listeners_public,
        listeners_conflicts,
        listeners_orphans,
        workloads_by_runtime: runtimes.into_iter().collect(),
        warnings_by_severity: severities.into_iter().collect(),
        tracked_runs: snapshot.tracked_runs.len(),
        events_dropped: snapshot
            .metadata
            .as_ref()
            .and_then(|m| m.events_dropped)
            .unwrap_or(0),
        event_rate,
        adapters,
    }
}

fn listener_is_tracked(listener: &lazyadmin_core::model::Listener, snapshot: &Snapshot) -> bool {
    listener.owners.iter().any(|owner| match owner {
        EntityRef::Run(_) => true,
        EntityRef::Process(key) => snapshot
            .processes
            .iter()
            .any(|process| &process.key == key && process.lazyadmin_run_id.is_some()),
        EntityRef::Workload(id) => snapshot
            .workloads
            .iter()
            .any(|workload| &workload.id == id && workload.lazyadmin_run_id.is_some()),
        _ => false,
    })
}

fn listener_is_project_member(
    listener: &lazyadmin_core::model::Listener,
    snapshot: &Snapshot,
) -> bool {
    if listener
        .owners
        .iter()
        .any(|owner| matches!(owner, EntityRef::Project(_)))
    {
        return true;
    }
    snapshot.workloads.iter().any(|workload| {
        workload.project.is_some()
            && (workload.listeners.iter().any(|id| id == &listener.id)
                || listener
                    .owners
                    .iter()
                    .any(|owner| matches!(owner, EntityRef::Workload(id) if id == &workload.id))
                || listener.owners.iter().any(|owner| match owner {
                    EntityRef::Process(key) => workload.pids.iter().any(|pid| pid == key),
                    _ => false,
                }))
    })
}

fn listener_owner_label(listener: &lazyadmin_core::model::Listener, snapshot: &Snapshot) -> String {
    listener
        .owners
        .iter()
        .find_map(|owner| match owner {
            EntityRef::Workload(id) => snapshot
                .workloads
                .iter()
                .find(|workload| &workload.id == id)
                .map(|workload| compact_text(&workload.display_name, 38)),
            EntityRef::Process(key) => snapshot
                .processes
                .iter()
                .find(|process| &process.key == key)
                .map(process_owner_label)
                .or_else(|| Some(format!("pid {}", key.pid))),
            EntityRef::Manager(id) => snapshot
                .managers
                .iter()
                .find(|manager| &manager.id == id)
                .map(|manager| compact_text(&manager.name, 38)),
            EntityRef::Project(id) => snapshot
                .projects
                .iter()
                .find(|project| &project.id == id)
                .map(|project| compact_text(&project.name, 38)),
            EntityRef::Run(id) => Some(format!("run {}", short_id(&id.to_string()))),
            EntityRef::Listener(id) => Some(format!("listener {}", short_id(&id.to_string()))),
            EntityRef::Action(id) => Some(format!("action {}", short_id(&id.to_string()))),
        })
        .unwrap_or_else(|| "unowned".into())
}

fn listener_runtime_label(
    listener: &lazyadmin_core::model::Listener,
    snapshot: &Snapshot,
    is_system: bool,
) -> String {
    if let Some(label) = listener.owners.iter().find_map(|owner| match owner {
        EntityRef::Workload(id) => snapshot
            .workloads
            .iter()
            .find(|workload| &workload.id == id)
            .map(|workload| runtime_kind_label(&workload.runtime)),
        EntityRef::Manager(id) => snapshot
            .managers
            .iter()
            .find(|manager| &manager.id == id)
            .map(|manager| runtime_kind_label(&manager.kind)),
        EntityRef::Process(key) => snapshot
            .processes
            .iter()
            .find(|process| &process.key == key)
            .map(process_runtime_label),
        _ => None,
    }) {
        return label;
    }
    if is_system {
        "systemd".into()
    } else {
        "direct".into()
    }
}

fn process_owner_label(process: &lazyadmin_core::model::Process) -> String {
    let command = process
        .exe
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .or_else(|| {
            process.cmdline.first().map(|cmd| {
                std::path::Path::new(cmd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(cmd)
                    .to_string()
            })
        })
        .filter(|cmd| !cmd.trim().is_empty())
        .unwrap_or_else(|| "process".into());
    compact_text(&format!("{command} pid {}", process.pid), 38)
}

fn process_runtime_label(process: &lazyadmin_core::model::Process) -> String {
    if process.systemd_unit.is_some() {
        "systemd".into()
    } else if process.container_id.is_some() {
        "container".into()
    } else if process.lazyadmin_run_id.is_some() {
        "tracked".into()
    } else {
        "direct".into()
    }
}

fn runtime_kind_label(kind: &lazyadmin_core::model::RuntimeKind) -> String {
    match kind {
        lazyadmin_core::model::RuntimeKind::Direct => "direct",
        lazyadmin_core::model::RuntimeKind::LazyadminTracked => "tracked",
        lazyadmin_core::model::RuntimeKind::SystemdSystem
        | lazyadmin_core::model::RuntimeKind::SystemdUser
        | lazyadmin_core::model::RuntimeKind::SystemdSocket => "systemd",
        lazyadmin_core::model::RuntimeKind::Docker => "docker",
        lazyadmin_core::model::RuntimeKind::DockerCompose => "compose",
        lazyadmin_core::model::RuntimeKind::Portless => "portless",
        lazyadmin_core::model::RuntimeKind::Podman => "podman",
        lazyadmin_core::model::RuntimeKind::PodmanCompose => "podman-compose",
        lazyadmin_core::model::RuntimeKind::PodmanPod => "podman-pod",
        lazyadmin_core::model::RuntimeKind::KubectlPortForward => "kubectl",
        lazyadmin_core::model::RuntimeKind::SshTunnel => "ssh",
        lazyadmin_core::model::RuntimeKind::Cloudflared => "cloudflared",
        lazyadmin_core::model::RuntimeKind::Socat => "socat",
        lazyadmin_core::model::RuntimeKind::Supervisor => "supervisor",
        lazyadmin_core::model::RuntimeKind::Launchd => "launchd",
        lazyadmin_core::model::RuntimeKind::Unknown => "unknown",
    }
    .into()
}

fn exposure_label(exposure: &Exposure) -> String {
    match exposure {
        Exposure::Loopback => "loopback",
        Exposure::LanOrPublic => "lan/public",
        Exposure::Public => "public",
        Exposure::ContainerOnly => "container",
        Exposure::UnixLocal => "unix",
        Exposure::Unknown => "unknown",
    }
    .into()
}

fn short_id(value: &str) -> String {
    compact_text(value, 12)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", value.chars().take(keep).collect::<String>())
}

fn compact_words(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let prefix: String = value.chars().take(keep).collect();
    let trimmed = prefix
        .rfind(char::is_whitespace)
        .map(|idx| prefix[..idx].trim_end().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(prefix);
    format!("{trimmed}…")
}
fn inspector_for_row(snapshot: &Snapshot, row: &RowVm) -> InspectorVm {
    InspectorView::lookup(snapshot, "listener", &row.id)
        .map(inspector_vm_from_view)
        .unwrap_or_else(|| {
            plain_inspector("No selection", "Listener vanished from the latest snapshot")
        })
}

fn inspector_for_process(snapshot: &Snapshot, key: &ProcessKey) -> Option<InspectorVm> {
    let id = serde_json::to_string(key).ok()?;
    InspectorView::lookup(snapshot, "process", &id).map(inspector_vm_from_view)
}

fn inspector_vm_from_view(view: InspectorView) -> InspectorVm {
    let title = match &view {
        InspectorView::Process(process) => {
            format!("pid {} — {}", process.identity.pid, process.title)
        }
        _ => view.title().to_string(),
    };
    let kind = view.kind().to_string();
    let sections: Vec<InspectorSectionVm> =
        view.to_sections().into_iter().map(Into::into).collect();
    let jump_targets = inspector_jump_targets(&sections);
    let lines = sections_to_plain_lines(&sections);
    let diagnostic_markdown = inspector_diagnostic_markdown(&title, &kind, &sections);
    InspectorVm {
        title,
        lines,
        provenance: vec![format!(
            "lazyadmin_runtime::view_model::InspectorView::{kind}"
        )],
        provenance_expanded: false,
        diagnostic_markdown,
        sections,
        jump_targets,
    }
}

fn inspector_jump_targets(sections: &[InspectorSectionVm]) -> Vec<JumpTarget> {
    sections
        .iter()
        .flat_map(|section| {
            section
                .rows
                .iter()
                .filter(move |row| row_receives_shortcut(&section.heading, row))
        })
        .filter_map(|row| row.jump_target.clone())
        .take(9)
        .collect()
}

fn related_listener_ids(sections: &[InspectorSectionVm]) -> Vec<String> {
    sections
        .iter()
        .filter(|section| section.heading == "RELATED")
        .flat_map(|section| section.rows.iter())
        .filter_map(|row| match &row.jump_target {
            Some(JumpTarget::Listener { id }) => Some(id.to_string()),
            _ => None,
        })
        .collect()
}

fn row_receives_shortcut(heading: &str, row: &InspectorRow) -> bool {
    row.jump_target.is_some()
        && !matches!(
            heading,
            "IDENTITY" | "CONFIDENCE" | "ACTIONS" | "WARNING GROUP"
        )
        && !(heading == "PROCESS" && row.label == "pid")
}

fn sections_to_plain_lines(sections: &[InspectorSectionVm]) -> Vec<String> {
    let mut lines = Vec::new();
    for section in sections {
        lines.push(section.heading.to_string());
        for row in &section.rows {
            let mut value = row.value.clone();
            if let Some(secondary) = &row.secondary {
                value.push_str("  ");
                value.push_str(secondary);
            }
            lines.push(format!("  {}: {}", row.label, value));
        }
        lines.push(String::new());
    }
    lines
}

fn inspector_diagnostic_markdown(
    title: &str,
    kind: &str,
    sections: &[InspectorSectionVm],
) -> String {
    let mut out = format!("# lazyadmin inspector\n\n- title: {title}\n- kind: {kind}\n");
    for section in sections {
        out.push_str(&format!("\n## {}\n", section.heading));
        for row in &section.rows {
            out.push_str(&format!("- {}: {}", row.label, row.value));
            if let Some(secondary) = &row.secondary {
                out.push_str(&format!(" ({secondary})"));
            }
            out.push('\n');
        }
    }
    out
}

fn plain_inspector(title: impl Into<String>, line: impl Into<String>) -> InspectorVm {
    let title = title.into();
    let line = line.into();
    InspectorVm {
        title: title.clone(),
        lines: vec![line.clone()],
        provenance: vec![],
        provenance_expanded: false,
        diagnostic_markdown: format!("# {title}\n\n{line}\n"),
        sections: vec![],
        jump_targets: vec![],
    }
}
fn groups(_show_system: bool) -> Vec<String> {
    RAIL_ENTRIES
        .iter()
        .map(|entry| entry.label.to_string())
        .collect()
}

pub fn key_to_command(key: KeyEvent) -> Option<Command> {
    let defaults = ResolvedKeybindings {
        bindings: ResolvedKeybindings::default_map()
            .into_iter()
            .map(|(a, b)| (a.as_name().into(), b))
            .collect(),
    };
    key_to_command_with_bindings(key, &defaults)
}

pub fn key_to_command_with_bindings(
    key: KeyEvent,
    keybindings: &ResolvedKeybindings,
) -> Option<Command> {
    let pressed = key_event_to_spec(key)?;
    for (action, specs) in &keybindings.bindings {
        if specs
            .iter()
            .any(|spec| normalize_key_spec(spec) == normalize_key_spec(&pressed))
        {
            return KeybindAction::parse(action).and_then(action_to_command);
        }
    }
    None
}

fn action_to_command(action: KeybindAction) -> Option<Command> {
    Some(match action {
        KeybindAction::Quit => Command::Quit,
        KeybindAction::Help => Command::Help,
        KeybindAction::NextPane => Command::NextPane,
        KeybindAction::PrevPane => Command::PrevPane,
        KeybindAction::OpenPalette => Command::Palette,
        KeybindAction::Filter | KeybindAction::ToggleFilter => Command::Filter,
        KeybindAction::ToggleSystem => Command::ToggleSystem,
        KeybindAction::Inspect => Command::Inspect,
        KeybindAction::Logs => Command::Logs,
        KeybindAction::Ports => Command::Ports,
        KeybindAction::ProcessTree => Command::Tree,
        KeybindAction::Metrics => Command::Metrics,
        KeybindAction::Restart => Command::Restart,
        KeybindAction::Stop => Command::Stop,
        KeybindAction::FreePort => Command::Free,
        KeybindAction::Kill => Command::Kill,
        KeybindAction::Open => Command::Open,
        KeybindAction::Edit => Command::Edit,
        KeybindAction::CopyDiagnostic => Command::CopyDiagnostic,
        KeybindAction::Run => Command::Run,
        KeybindAction::Refresh => Command::Refresh,
    })
}

fn key_event_to_spec(key: KeyEvent) -> Option<String> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            return Some(format!("ctrl+{}", c.to_ascii_lowercase()));
        }
    }
    match key.code {
        KeyCode::Char(c) => Some(c.to_string()),
        KeyCode::Tab => Some("tab".into()),
        KeyCode::BackTab => Some("shift+tab".into()),
        KeyCode::Enter => Some("enter".into()),
        KeyCode::Esc => Some("esc".into()),
        KeyCode::Up => Some("up".into()),
        KeyCode::Down => Some("down".into()),
        KeyCode::Left => Some("left".into()),
        KeyCode::Right => Some("right".into()),
        KeyCode::F(5) => Some("f5".into()),
        _ => None,
    }
}

fn normalize_key_spec(spec: &str) -> String {
    if spec.chars().count() == 1 {
        spec.to_string()
    } else {
        spec.to_ascii_lowercase()
    }
}

#[allow(dead_code)]
fn key_to_command_hardcoded_legacy(key: KeyEvent) -> Option<Command> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('/'), _) => Some(Command::Filter),
        (KeyCode::Char(':'), _) => Some(Command::Palette),
        (KeyCode::Tab, _) => Some(Command::NextPane),
        (KeyCode::BackTab, _) => Some(Command::PrevPane),
        (KeyCode::Enter, _) => Some(Command::Inspect),
        (KeyCode::Char('l'), _) => Some(Command::Logs),
        (KeyCode::Char('p'), _) => Some(Command::Ports),
        (KeyCode::Char('t'), _) => Some(Command::Tree),
        (KeyCode::Char('m'), _) => Some(Command::Metrics),
        (KeyCode::Char('r'), _) => Some(Command::Restart),
        (KeyCode::Char('s'), _) => Some(Command::Stop),
        (KeyCode::Char('f'), _) => Some(Command::Free),
        (KeyCode::Char('k'), _) => Some(Command::Kill),
        (KeyCode::Char('o'), _) => Some(Command::Open),
        (KeyCode::Char('e'), _) => Some(Command::Edit),
        (KeyCode::Char('y'), _) => Some(Command::CopyDiagnostic),
        (KeyCode::Char('S'), _) => Some(Command::ToggleSystem),
        (KeyCode::Char('R'), _) => Some(Command::Run),
        (KeyCode::Char('?'), _) => Some(Command::Help),
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            Some(Command::Quit)
        }
        _ => None,
    }
}

pub fn palette_entries(filter: &str) -> Vec<&'static str> {
    let all = [
        "open",
        "logs",
        "restart",
        "stop",
        "free-port",
        "pause-restart",
        "kill",
        "copy-diagnostic",
        "show-process-tree",
        "show-cgroup",
        "show-network-namespace",
        "edit-unit",
        "edit-compose-file",
        "open-project",
        "refresh",
        "overview",
        "listeners",
        "workloads",
        "processes",
        "view all",
        "everything",
        "public",
        "conflicts",
        "projects",
        "doctor",
        "process-tree",
        "metrics",
        "theme default-dark",
        "theme night-owl",
        "theme night-owl-light",
        "theme high-contrast",
        "theme colorblind-safe",
        "theme solarized-dark",
        "theme default-light",
        "reload",
        "export-json",
        "diff",
        "doctor",
        "toggle-system-services",
        "runs-list",
        "run-stop",
        "run-restart",
        "run-forget",
    ];
    if filter.is_empty() {
        return all.to_vec();
    }
    let m = SkimMatcherV2::default();
    all.into_iter()
        .filter(|e| m.fuzzy_match(e, filter).is_some())
        .collect()
}

pub fn render(view_model: &ViewModel, frame: &mut ratatui::Frame<'_>, area: Rect, theme: &Theme) {
    render_view_kind(
        view_model,
        frame,
        area,
        theme,
        RenderContext {
            view: ViewKind::Overview,
            active_pane: Pane::Rows,
            keybindings: None,
            selected_row: 0,
            overview_hint_visible: false,
            listener_filter: ListenerFilter::All,
            listeners_hint_visible: false,
            related_listener_filter: None,
        },
    );
}

fn panel_block(title: impl Into<String>, theme: &Theme, active: bool) -> Block<'static> {
    let border = if active {
        theme.accent.color()
    } else {
        theme.footer.color()
    };
    Block::default()
        .title(format!(" {} ", title.into()))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).bg(theme.base_bg.color()))
        .style(
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        )
}

pub fn parse_view_kind(value: &str) -> Option<ViewKind> {
    match value.to_ascii_lowercase().replace(['-', '_'], " ").as_str() {
        "overview" | "digest" => Some(ViewKind::Overview),
        "listeners" => Some(ViewKind::Listeners),
        "workloads" => Some(ViewKind::Workloads),
        "processes" => Some(ViewKind::Processes),
        "everything" | "all" => Some(ViewKind::Everything),
        "ports" => Some(ViewKind::Ports),
        "public" | "public listeners" => Some(ViewKind::Public),
        "conflicts" => Some(ViewKind::Conflicts),
        "projects" => Some(ViewKind::Projects),
        "managers" => Some(ViewKind::Managers),
        "orphans" => Some(ViewKind::Orphans),
        "tracked runs" | "runs" => Some(ViewKind::TrackedRuns),
        "logs" => Some(ViewKind::Logs),
        "doctor" | "warnings" => Some(ViewKind::Doctor),
        "process tree" | "tree" => Some(ViewKind::ProcessTree),
        "metrics" => Some(ViewKind::Metrics),
        _ => None,
    }
}

fn title_for_view(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Overview => "Overview",
        ViewKind::Listeners => "Listeners",
        ViewKind::Workloads => "Workloads",
        ViewKind::Processes => "Processes",
        ViewKind::Ports => "Ports",
        ViewKind::Public => "Public",
        ViewKind::Conflicts => "Conflicts",
        ViewKind::Projects => "Projects",
        ViewKind::Managers => "Managers",
        ViewKind::Orphans => "Orphans",
        ViewKind::TrackedRuns => "Tracked Runs",
        ViewKind::Logs => "Logs",
        ViewKind::Doctor => "Doctor",
        ViewKind::ProcessTree => "Process Tree",
        ViewKind::Metrics => "Metrics",
        ViewKind::Everything => "Everything",
    }
}

fn canonical_rail_view(view: ViewKind) -> ViewKind {
    match view {
        ViewKind::Everything
        | ViewKind::Ports
        | ViewKind::Public
        | ViewKind::Conflicts
        | ViewKind::Orphans
        | ViewKind::TrackedRuns => ViewKind::Listeners,
        ViewKind::Projects | ViewKind::Managers => ViewKind::Workloads,
        ViewKind::Logs | ViewKind::ProcessTree => ViewKind::Processes,
        other => other,
    }
}

fn group_is_active(group: &str, view: ViewKind) -> bool {
    matches!(
        (group, canonical_rail_view(view)),
        ("Overview", ViewKind::Overview)
            | ("Listeners", ViewKind::Listeners)
            | ("Workloads", ViewKind::Workloads)
            | ("Processes", ViewKind::Processes)
            | ("Doctor", ViewKind::Doctor)
            | ("Metrics", ViewKind::Metrics)
    )
}

/// CLI command(s) the user can run instead when the TUI refuses to render at
/// the current width. Issue #6 acceptance criterion: the refusal screen must
/// list at least one matching CLI command for the active view.
fn cli_hints_for_view(view: ViewKind) -> &'static [&'static str] {
    match view {
        ViewKind::Overview => &["lazyadmin overview --json"],
        ViewKind::Listeners => &["lazyadmin ps --json", "lazyadmin public --json"],
        ViewKind::Workloads => &["lazyadmin projects --json"],
        ViewKind::Processes => &["lazyadmin ps --json"],
        ViewKind::Everything => &["lazyadmin ps --json", "lazyadmin export --json"],
        ViewKind::Ports => &["lazyadmin ps --json"],
        ViewKind::Public => &["lazyadmin public --json"],
        ViewKind::Conflicts => &["lazyadmin conflicts --json"],
        ViewKind::Orphans => &["lazyadmin doctor --json"],
        ViewKind::TrackedRuns => &["lazyadmin export --json"],
        ViewKind::Projects => &["lazyadmin projects --json"],
        ViewKind::Logs => &["lazyadmin logs"],
        ViewKind::Doctor => &["lazyadmin doctor --json"],
        ViewKind::ProcessTree => &["lazyadmin ps --json"],
        ViewKind::Metrics => &["lazyadmin export --json"],
        ViewKind::Managers => &["lazyadmin ps --json"],
    }
}

fn narrow_refusal_message(view: ViewKind) -> String {
    let hints = cli_hints_for_view(view);
    let formatted = hints
        .iter()
        .map(|cmd| format!("`{cmd}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "lazyadmin TUI needs 60+ columns to show the {view_title} view.\n\
         Try {formatted} or widen the terminal.",
        view_title = title_for_view(view),
    )
}

fn group_view_kind(group: &str) -> Option<ViewKind> {
    RAIL_ENTRIES
        .iter()
        .find(|entry| entry.label == group)
        .and_then(|entry| parse_view_kind(entry.id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExposureSignal {
    Public,
    Lan,
    Loopback,
}

fn row_exposure_signal(row: &RowVm) -> ExposureSignal {
    match row.exposure.as_str() {
        "public" => ExposureSignal::Public,
        "lan/public" => ExposureSignal::Lan,
        _ => ExposureSignal::Loopback,
    }
}

fn signal_color(theme: &Theme, color: Color) -> Color {
    if theme.fallback_palette == PaletteMode::Monochrome {
        theme.base_fg.color()
    } else {
        color
    }
}

fn exposure_signal_color(signal: ExposureSignal, theme: &Theme) -> Color {
    match signal {
        ExposureSignal::Public => signal_color(theme, theme.risk_public.color()),
        ExposureSignal::Lan => signal_color(theme, theme.risk_lan.color()),
        ExposureSignal::Loopback => signal_color(theme, theme.risk_loopback.color()),
    }
}

fn exposure_signal_glyph(signal: ExposureSignal) -> &'static str {
    match signal {
        ExposureSignal::Public => "●",
        ExposureSignal::Lan => "◐",
        ExposureSignal::Loopback => " ",
    }
}

fn row_marker_glyph(row: &RowVm) -> &'static str {
    if row.is_conflict {
        "┃"
    } else if row.is_tracked || row.is_project {
        "▎"
    } else {
        " "
    }
}

fn row_marker_color(row: &RowVm, theme: &Theme) -> Color {
    if row.is_conflict {
        signal_color(theme, theme.marker_conflict.color())
    } else if row.is_tracked {
        signal_color(theme, theme.marker_tracked.color())
    } else if row.is_project {
        signal_color(theme, theme.marker_project.color())
    } else {
        theme.footer.color()
    }
}

fn marker_glyph(is_conflict: bool, is_tracked: bool, is_project: bool) -> &'static str {
    if is_conflict {
        "┃"
    } else if is_tracked || is_project {
        "▎"
    } else {
        " "
    }
}

fn marker_color(is_conflict: bool, is_tracked: bool, is_project: bool, theme: &Theme) -> Color {
    if is_conflict {
        signal_color(theme, theme.marker_conflict.color())
    } else if is_tracked {
        signal_color(theme, theme.marker_tracked.color())
    } else if is_project {
        signal_color(theme, theme.marker_project.color())
    } else {
        theme.footer.color()
    }
}

fn row_signal_cell(row: &RowVm, theme: &Theme) -> Cell<'static> {
    let signal = row_exposure_signal(row);
    let marker = row_marker_glyph(row);
    Cell::from(Line::from(vec![
        Span::styled(
            marker.to_string(),
            Style::default()
                .fg(row_marker_color(row, theme))
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            exposure_signal_glyph(signal).to_string(),
            Style::default()
                .fg(exposure_signal_color(signal, theme))
                .bg(theme.base_bg.color())
                .add_modifier(match signal {
                    ExposureSignal::Public => Modifier::BOLD,
                    ExposureSignal::Lan | ExposureSignal::Loopback => Modifier::empty(),
                }),
        ),
    ]))
}

fn marker_cell(
    is_conflict: bool,
    is_tracked: bool,
    is_project: bool,
    theme: &Theme,
) -> Cell<'static> {
    Cell::from(Span::styled(
        marker_glyph(is_conflict, is_tracked, is_project),
        Style::default()
            .fg(marker_color(is_conflict, is_tracked, is_project, theme))
            .bg(theme.base_bg.color()),
    ))
}

fn process_signal_cell(row: &ProcessTreeRow, theme: &Theme) -> Cell<'static> {
    let alert = if row.warnings.is_empty() { " " } else { "⚠" };
    Cell::from(Line::from(vec![
        Span::styled(
            marker_glyph(false, row.is_tracked, row.is_project),
            Style::default()
                .fg(marker_color(false, row.is_tracked, row.is_project, theme))
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            alert,
            Style::default()
                .fg(signal_color(theme, theme.pip_warn.color()))
                .bg(theme.base_bg.color())
                .add_modifier(if row.warnings.is_empty() {
                    Modifier::empty()
                } else {
                    Modifier::BOLD
                }),
        ),
    ]))
}

fn listener_signal_counts(rows: &[RowVm]) -> (usize, usize, usize) {
    rows.iter().fold((0, 0, 0), |mut counts, row| {
        match row_exposure_signal(row) {
            ExposureSignal::Public => counts.0 += 1,
            ExposureSignal::Lan => counts.1 += 1,
            ExposureSignal::Loopback => counts.2 += 1,
        }
        counts
    })
}

fn pad_to_width(mut line: Line<'static>, width: u16) -> Line<'static> {
    let width = width as usize;
    let current = line.width();
    if current < width {
        line.spans.push(Span::raw(" ".repeat(width - current)));
    }
    line
}

fn header_pip_spans(pip: &HeaderPip, theme: &Theme) -> Vec<Span<'static>> {
    let dropped = pip.drops.as_ref().map(|drops| drops.dropped).unwrap_or(0);
    let stale = pip.freshness.age_seconds > 5;
    let degraded = pip.adapters.degraded > 0;
    let (glyph, label, color) = if dropped > 0 {
        (
            "⚠",
            format!("events dropped {dropped}"),
            theme.pip_error.color(),
        )
    } else if stale {
        (
            "●",
            format!("refresh stale ({}s)", pip.freshness.age_seconds),
            theme.pip_warn.color(),
        )
    } else if degraded {
        ("●", "degraded".to_string(), theme.pip_warn.color())
    } else {
        ("●", "healthy".to_string(), theme.pip_ok.color())
    };
    let color = signal_color(theme, color);
    let mut spans = vec![
        Span::styled(
            format!("  {glyph} {label}"),
            Style::default()
                .fg(color)
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  adapters: {}/{} active",
                pip.adapters.active, pip.adapters.total
            ),
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color()),
        ),
    ];
    if dropped == 0 {
        spans.push(Span::styled(
            format!("  last update {}s ago", pip.freshness.age_seconds),
            Style::default()
                .fg(if stale {
                    signal_color(theme, theme.pip_warn.color())
                } else {
                    theme.footer.color()
                })
                .bg(theme.base_bg.color()),
        ));
    }
    spans
}

fn render_header(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    view: ViewKind,
) {
    let total = view_model.rows.len();
    let (public, lan, loopback) = listener_signal_counts(&view_model.rows);
    let inactive = Style::default()
        .fg(theme.footer.color())
        .bg(theme.base_bg.color());
    let mut spans = vec![
        Span::styled(
            "lazyadmin",
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}  ", title_for_view(view)),
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{total} listeners"), inactive),
        Span::styled(
            format!("  {public} public"),
            Style::default()
                .fg(if public > 0 {
                    signal_color(theme, theme.risk_public.color())
                } else {
                    theme.footer.color()
                })
                .bg(theme.base_bg.color())
                .add_modifier(if public > 0 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("  {lan} LAN"),
            Style::default()
                .fg(if lan > 0 {
                    signal_color(theme, theme.risk_lan.color())
                } else {
                    theme.footer.color()
                })
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            format!("  {loopback} loopback"),
            Style::default()
                .fg(if loopback > 0 {
                    signal_color(theme, theme.risk_loopback.color())
                } else {
                    theme.footer.color()
                })
                .bg(theme.base_bg.color()),
        ),
    ];
    spans.extend(header_pip_spans(&view_model.header_pip, theme));
    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(theme.selection.color())),
            )
            .style(Style::default().bg(theme.base_bg.color())),
        area,
    );
}

#[derive(Clone, Copy)]
struct RenderContext<'a> {
    view: ViewKind,
    active_pane: Pane,
    keybindings: Option<&'a ResolvedKeybindings>,
    selected_row: usize,
    overview_hint_visible: bool,
    listener_filter: ListenerFilter,
    listeners_hint_visible: bool,
    related_listener_filter: Option<&'a RelatedListenerFilter>,
}

fn render_view_kind(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    ctx: RenderContext<'_>,
) {
    tracing::debug!("tui.render");
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.base_bg.color())),
        area,
    );
    if view_model.layout == LayoutMode::Refuse || area.width < 60 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        if ctx.view == ViewKind::Overview {
            render_digest_refuse(view_model, frame, chunks[0], theme);
            render_footer_hints(view_model, frame, chunks[1], theme, ctx);
            return;
        }
        let p = Paragraph::new(narrow_refusal_message(ctx.view))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false })
            .style(
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            )
            .block(panel_block("lazyadmin", theme, true));
        frame.render_widget(p, chunks[0]);
        render_footer_hints(view_model, frame, chunks[1], theme, ctx);
        return;
    }
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    render_header(view_model, frame, vertical[0], theme, ctx.view);
    let body = vertical[1];
    let chunks = match view_model.layout {
        LayoutMode::ThreePane => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(NAV_PANE_WIDTH),
                Constraint::Min(MAIN_PANE_MIN_WIDTH),
                Constraint::Length(INSPECTOR_PANE_WIDTH),
            ])
            .split(body),
        _ => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(body),
    };
    if view_model.layout == LayoutMode::ThreePane {
        let groups = List::new(
            view_model
                .groups
                .iter()
                .map(|g| {
                    let active = group_is_active(g, ctx.view);
                    let navigable = group_view_kind(g).is_some();
                    let marker = if active {
                        "› "
                    } else if navigable {
                        "  "
                    } else {
                        "· "
                    };
                    let label_style = if active {
                        Style::default()
                            .fg(theme.base_fg.color())
                            .bg(theme.base_bg.color())
                            .add_modifier(Modifier::BOLD)
                    } else if navigable {
                        Style::default()
                            .fg(theme.footer.color())
                            .bg(theme.base_bg.color())
                    } else {
                        Style::default()
                            .fg(theme.info.color())
                            .bg(theme.base_bg.color())
                            .add_modifier(Modifier::DIM)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            marker,
                            Style::default()
                                .fg(theme.accent.color())
                                .bg(theme.base_bg.color()),
                        ),
                        Span::styled(g.clone(), label_style),
                    ]))
                })
                .collect::<Vec<_>>(),
        )
        .block(panel_block("Views", theme, ctx.active_pane == Pane::Groups))
        .style(Style::default().bg(theme.base_bg.color()));
        frame.render_widget(groups, chunks[0]);
        render_main_pane(
            view_model,
            frame,
            chunks[1],
            theme,
            ctx,
            ctx.active_pane == Pane::Rows,
        );
        render_inspector(
            view_model,
            frame,
            chunks[2],
            theme,
            ctx.active_pane == Pane::Inspector,
        );
    } else {
        render_main_pane(view_model, frame, chunks[0], theme, ctx, true);
    }
    render_footer_hints(view_model, frame, vertical[2], theme, ctx);
}

fn footer_hint_line(
    view_model: &ViewModel,
    ctx: RenderContext<'_>,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        match ctx.active_pane {
            Pane::Groups => "[↑↓] views   [enter] open   [tab] pane",
            Pane::Rows => "[?] help   [:] palette   [/] filter   [enter] inspect   [q] quit",
            Pane::Inspector => "[1-9] jump   [v] view related   [tab] pane   [q] quit",
        },
        Style::default()
            .fg(theme.footer.color())
            .bg(theme.base_bg.color()),
    )];
    if view_model.hidden_system_count > 0 {
        spans.push(Span::styled(
            format!(
                "   hidden system: {}   [S] show",
                view_model.hidden_system_count
            ),
            Style::default()
                .fg(theme.system_noise.color())
                .bg(theme.base_bg.color()),
        ));
    }
    Line::from(spans)
}

fn render_footer_hints(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    ctx: RenderContext<'_>,
) {
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.base_bg.color())),
        area,
    );
    frame.render_widget(
        Paragraph::new(pad_to_width(
            footer_hint_line(view_model, ctx, theme),
            area.width,
        ))
        .style(
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ),
        area,
    );
}

fn render_main_pane(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    ctx: RenderContext<'_>,
    active: bool,
) {
    match ctx.view {
        ViewKind::Overview => render_digest(
            view_model,
            frame,
            area,
            theme,
            active,
            ctx.selected_row,
            ctx.overview_hint_visible,
        ),
        ViewKind::ProcessTree | ViewKind::Processes => {
            render_process_tree(view_model, frame, area, theme, active)
        }
        ViewKind::Metrics => render_metrics(view_model, frame, area, theme, active),
        ViewKind::Logs => render_logs(view_model, frame, area, theme, active),
        ViewKind::Doctor => {
            render_doctor_view(view_model, frame, area, theme, active, ctx.selected_row)
        }
        ViewKind::Conflicts => render_summary_table(
            &view_model.conflicts,
            "Conflicts",
            "No listener conflicts detected.",
            frame,
            area,
            theme,
            active,
        ),
        ViewKind::Orphans => render_summary_table(
            &view_model.orphans,
            "Orphans",
            "No orphan listeners or routes detected.",
            frame,
            area,
            theme,
            active,
        ),
        ViewKind::Projects => render_summary_table(
            &view_model.projects,
            "Projects",
            "No projects discovered. Configure project roots or run discovery.",
            frame,
            area,
            theme,
            active,
        ),
        ViewKind::Workloads => render_summary_table(
            &view_model.workloads,
            "Workloads",
            "No workloads discovered yet.",
            frame,
            area,
            theme,
            active,
        ),
        ViewKind::TrackedRuns => render_summary_table(
            &view_model.tracked_runs,
            "Tracked Runs",
            "No lazyadmin tracked runs are active.",
            frame,
            area,
            theme,
            active,
        ),
        _ => render_rows_table(
            view_model,
            frame,
            area,
            theme,
            ctx.view,
            ctx.selected_row,
            active,
            ctx.listener_filter,
            ctx.listeners_hint_visible,
            ctx.related_listener_filter,
        ),
    }
    if let Some(keybindings) = ctx.keybindings {
        let _ = help_lines(keybindings);
    }
}

fn render_digest(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
    selected_index: usize,
    overview_hint_visible: bool,
) {
    let digest = &view_model.digest;
    let mut lines = Vec::new();
    if overview_hint_visible {
        lines.push(Line::from(Span::styled(
            "New layout: this is the digest. Press [v] for the full Listeners table.",
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(vec![
        Span::styled(
            "EXPOSED ",
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "{} public · {} LAN · {} unowned ({} shown)",
                digest.exposed.total_public,
                digest.exposed.total_lan,
                digest.exposed.unowned_count,
                digest.exposed.rows.len()
            ),
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        ),
    ]));
    if digest.exposed.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", digest.exposed.empty_copy),
            Style::default()
                .fg(theme.ok.color())
                .bg(theme.base_bg.color()),
        )));
    } else {
        for row in &digest.exposed.rows {
            let extra = if row.extra_ports > 0 {
                format!(" +{} ports", row.extra_ports)
            } else {
                String::new()
            };
            let (glyph, color) = if matches!(row.exposure, Exposure::Public) {
                ("●", signal_color(theme, theme.risk_public.color()))
            } else {
                ("◐", signal_color(theme, theme.risk_lan.color()))
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {glyph} {}  {}  {}{}",
                    row.bind,
                    row.owner_label,
                    row.project.clone().unwrap_or_else(|| "-".into()),
                    extra
                ),
                Style::default().fg(color).bg(theme.base_bg.color()),
            )));
        }
    }
    lines.push(digest_action_line(
        selected_index == 0,
        format!(
            "[view all {} →]",
            digest.exposed.total_public + digest.exposed.total_lan
        ),
        theme,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "CONFLICTS {} ({} shown)",
            digest.conflicts.total,
            digest.conflicts.rows.len()
        ),
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD),
    )));
    if digest.conflicts.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", digest.conflicts.empty_copy),
            Style::default()
                .fg(theme.ok.color())
                .bg(theme.base_bg.color()),
        )));
    } else {
        for row in &digest.conflicts.rows {
            lines.push(Line::from(Span::styled(
                format!(
                    "  ┃ {}  owners={}  {}",
                    row.bind, row.owner_count, row.reason
                ),
                Style::default()
                    .fg(signal_color(theme, theme.marker_conflict.color()))
                    .bg(theme.base_bg.color()),
            )));
        }
    }
    lines.push(digest_action_line(
        selected_index == 1,
        format!("[view all {} →]", digest.conflicts.total),
        theme,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "PROJECTS {} ({} shown)",
            digest.your_projects.total,
            digest.your_projects.rows.len()
        ),
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD),
    )));
    if digest.your_projects.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", digest.your_projects.empty_copy),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        )));
    } else {
        for row in &digest.your_projects.rows {
            lines.push(Line::from(Span::styled(
                format!(
                    "  ▎ {}  {} listeners  {}",
                    row.name, row.listener_count, row.root
                ),
                Style::default()
                    .fg(signal_color(theme, theme.marker_project.color()))
                    .bg(theme.base_bg.color()),
            )));
        }
    }
    lines.push(digest_action_line(
        selected_index == 2,
        format!("[view all {} →]", digest.your_projects.total),
        theme,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "TRIAGE {} actionable · {} noisy groups",
            digest.triage.summary.actionable, digest.triage.summary.noise_groups
        ),
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD),
    )));
    if digest.triage.summary.actionable == 0 {
        lines.push(Line::from(Span::styled(
            format!("  {}", digest.triage.empty_copy),
            Style::default()
                .fg(theme.ok.color())
                .bg(theme.base_bg.color()),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Press Doctor to review grouped warnings.",
            Style::default()
                .fg(theme.warning.color())
                .bg(theme.base_bg.color()),
        )));
    }
    lines.push(digest_action_line(
        selected_index == 3,
        format!("[view all {} →]", digest.triage.summary.actionable),
        theme,
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(panel_block("Overview", theme, active))
            .style(
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            ),
        area,
    );
}

fn digest_action_line(selected: bool, text: String, theme: &Theme) -> Line<'static> {
    let prefix = if selected { "  › " } else { "    " };
    let style = if selected {
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.footer.color())
            .bg(theme.base_bg.color())
    };
    Line::from(Span::styled(format!("{prefix}{text}"), style))
}

fn render_digest_refuse(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
) {
    let digest = &view_model.digest;
    let message = format!(
        "EXPOSED {} · CONFLICTS {} · PROJECTS {} · TRIAGE {} actionable\nTry `lazyadmin overview --json` or widen the terminal.",
        digest.exposed.total_public + digest.exposed.total_lan,
        digest.conflicts.total,
        digest.your_projects.total,
        digest.triage.summary.actionable
    );
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false })
            .block(panel_block("Overview", theme, true))
            .style(
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            ),
        area,
    );
}

fn render_empty_state(
    title: &str,
    message: &str,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false })
            .block(panel_block(title, theme, active))
            .style(
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            ),
        area,
    );
}

fn render_summary_table(
    rows: &[SummaryRowVm],
    title: &str,
    empty_message: &str,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    if rows.is_empty() {
        render_empty_state(title, empty_message, frame, area, theme, active);
        return;
    }
    let table_rows = rows.iter().map(|row| {
        let row_fg = if row.is_system {
            theme.system_noise.color()
        } else {
            theme.base_fg.color()
        };
        Row::new(vec![
            marker_cell(row.is_conflict, row.is_tracked, row.is_project, theme),
            Cell::from(Span::styled(
                compact_text(&row.name, 24),
                Style::default()
                    .fg(row_fg)
                    .bg(theme.base_bg.color())
                    .add_modifier(Modifier::BOLD),
            )),
            Cell::from(Span::styled(
                compact_text(&row.kind, 14),
                Style::default()
                    .fg(if row.is_system {
                        theme.system_noise.color()
                    } else {
                        theme.info.color()
                    })
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                compact_text(&row.state, 18),
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                compact_text(&row.details, 64),
                Style::default().fg(row_fg).bg(theme.base_bg.color()),
            )),
        ])
        .style(Style::default().bg(theme.base_bg.color()))
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),
            Constraint::Length(24),
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(["", "Name", "Kind", "State", "Details"]).style(
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(panel_block(title, theme, active))
    .style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.base_bg.color()),
    );
    frame.render_widget(table, area);
}

#[allow(clippy::too_many_arguments)]
fn render_rows_table(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    view: ViewKind,
    selected_row: usize,
    active: bool,
    listener_filter: ListenerFilter,
    listeners_hint_visible: bool,
    related_listener_filter: Option<&RelatedListenerFilter>,
) {
    let rows_area = if view == ViewKind::Listeners {
        render_listener_chips(
            view_model,
            frame,
            area,
            theme,
            listener_filter,
            listeners_hint_visible,
            related_listener_filter,
        )
    } else {
        area
    };
    let effective_filter = effective_listener_filter(view, listener_filter);
    let rows = view_model
        .rows
        .iter()
        .filter(|r| {
            row_matches_visible_listener_scope(r, view, effective_filter, related_listener_filter)
        })
        .enumerate()
        .map(|(idx, r)| {
            let quiet = if r.is_system {
                theme.system_noise.color()
            } else if idx % 2 == 0 {
                theme.base_fg.color()
            } else {
                theme.footer.color()
            };
            let owner_fg = if r.is_system {
                theme.system_noise.color()
            } else {
                theme.base_fg.color()
            };
            let runtime_fg = if r.is_system {
                theme.system_noise.color()
            } else {
                theme.info.color()
            };
            let exposure_signal = row_exposure_signal(r);
            let exposure_style = match exposure_signal {
                ExposureSignal::Public => Style::default()
                    .fg(signal_color(theme, theme.risk_public.color()))
                    .bg(theme.base_bg.color())
                    .add_modifier(Modifier::BOLD),
                ExposureSignal::Lan => Style::default()
                    .fg(signal_color(theme, theme.risk_lan.color()))
                    .bg(theme.base_bg.color()),
                ExposureSignal::Loopback => Style::default()
                    .fg(if r.is_system {
                        theme.system_noise.color()
                    } else {
                        theme.footer.color()
                    })
                    .bg(theme.base_bg.color()),
            };
            Row::new(vec![
                row_signal_cell(r, theme),
                Cell::from(Span::styled(
                    r.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                    Style::default()
                        .fg(if r.is_system {
                            theme.system_noise.color()
                        } else {
                            theme.accent.color()
                        })
                        .bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    compact_text(&r.bind, 22),
                    Style::default().fg(quiet).bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    r.owner.clone(),
                    Style::default().fg(owner_fg).bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    r.runtime.clone(),
                    Style::default().fg(runtime_fg).bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(r.exposure.clone(), exposure_style)),
            ])
            .style(Style::default().bg(theme.base_bg.color()))
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(5),
            Constraint::Length(14),
            Constraint::Min(12),
            Constraint::Length(9),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(["", "Port", "Bind", "Owner", "Runtime", "Scope"]).style(
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(panel_block(title_for_view(view), theme, active))
    .style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.base_bg.color()),
    )
    .row_highlight_style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.selection.color())
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    let row_count = view_model
        .rows
        .iter()
        .filter(|r| {
            row_matches_visible_listener_scope(r, view, effective_filter, related_listener_filter)
        })
        .count();
    if row_count > 0 {
        state.select(Some(selected_row.min(row_count.saturating_sub(1))));
    }
    frame.render_stateful_widget(table, rows_area, &mut state);
}

fn render_listener_chips(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active_filter: ListenerFilter,
    hint_visible: bool,
    related_listener_filter: Option<&RelatedListenerFilter>,
) -> Rect {
    let toolbar_height = if hint_visible || related_listener_filter.is_some() {
        3
    } else {
        2
    };
    if area.height <= toolbar_height {
        return area;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(toolbar_height), Constraint::Min(1)])
        .split(area);
    let filters = [
        ListenerFilter::All,
        ListenerFilter::Public,
        ListenerFilter::Conflicts,
        ListenerFilter::Orphans,
        ListenerFilter::Unowned,
        ListenerFilter::Tracked,
    ];
    let (public, lan, loopback) = listener_signal_counts(&view_model.rows);
    let count_spans = vec![
        Span::styled(
            "Exposure ",
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            format!("● {public} public  "),
            Style::default()
                .fg(signal_color(theme, theme.risk_public.color()))
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("◐ {lan} LAN  "),
            Style::default()
                .fg(signal_color(theme, theme.risk_lan.color()))
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            format!("{loopback} loopback"),
            Style::default()
                .fg(signal_color(theme, theme.risk_loopback.color()))
                .bg(theme.base_bg.color()),
        ),
    ];
    let mut spans = vec![Span::styled(
        "Filters ",
        Style::default()
            .fg(theme.footer.color())
            .bg(theme.base_bg.color()),
    )];
    for filter in filters {
        let count = view_model
            .rows
            .iter()
            .filter(|row| row_matches_listener_filter(row, filter))
            .count();
        let label = format!("[{} {}] ", filter.label(), count);
        let style = if filter == active_filter {
            Style::default()
                .fg(theme.base_bg.color())
                .bg(theme.accent.color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color())
        };
        spans.push(Span::styled(label, style));
    }
    let mut lines = vec![Line::from(count_spans), Line::from(spans)];
    if let Some(filter) = related_listener_filter {
        lines.push(Line::from(Span::styled(
            format!(
                "Related filter: {} listeners from {} — press [A]ll to clear.",
                filter.ids.len(),
                filter.label
            ),
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color()),
        )));
    } else if hint_visible {
        lines.push(Line::from(Span::styled(
            "Filters now live as chips — try [P]ublic, [C]onflicts, [/] to search.",
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.base_bg.color())),
        chunks[0],
    );
    chunks[1]
}

fn effective_listener_filter(view: ViewKind, listener_filter: ListenerFilter) -> ListenerFilter {
    match view {
        ViewKind::Public => ListenerFilter::Public,
        ViewKind::Conflicts => ListenerFilter::Conflicts,
        ViewKind::Orphans => ListenerFilter::Orphans,
        _ => listener_filter,
    }
}

fn row_matches_listener_filter(row: &RowVm, filter: ListenerFilter) -> bool {
    match filter {
        ListenerFilter::All => true,
        ListenerFilter::Public => row.badges.iter().any(|badge| badge == "PUBLIC"),
        ListenerFilter::Conflicts => row.is_conflict,
        ListenerFilter::Orphans | ListenerFilter::Unowned => row.is_orphan,
        ListenerFilter::Tracked => row.is_tracked,
    }
}

fn row_matches_view_filter(row: &RowVm, view: ViewKind, listener_filter: ListenerFilter) -> bool {
    match view {
        ViewKind::Ports => row.port.is_some(),
        ViewKind::Public | ViewKind::Conflicts | ViewKind::Orphans | ViewKind::Listeners => {
            row_matches_listener_filter(row, listener_filter)
        }
        _ => true,
    }
}

fn row_matches_visible_listener_scope(
    row: &RowVm,
    view: ViewKind,
    listener_filter: ListenerFilter,
    related_listener_filter: Option<&RelatedListenerFilter>,
) -> bool {
    row_matches_view_filter(row, view, listener_filter)
        && related_listener_filter.is_none_or(|filter| filter.ids.contains(&row.id))
}

fn render_inspector(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    let lines = if view_model.inspector.sections.is_empty() {
        render_plain_inspector_lines(&view_model.inspector, theme)
    } else {
        render_section_lines(&view_model.inspector.sections, theme)
    };
    let widget = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(panel_block(
            view_model.inspector.title.clone(),
            theme,
            active,
        ))
        .style(
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        );
    frame.render_widget(widget, area);
}

fn render_plain_inspector_lines<'a>(inspector: &'a InspectorVm, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = inspector
        .lines
        .iter()
        .map(|line| inspector_line(line, theme))
        .collect();
    if !inspector.provenance.is_empty() {
        lines.push(Line::from(""));
        lines.push(section_heading_line("Provenance", theme));
        lines.extend(inspector.provenance.iter().map(|p| {
            Line::from(Span::styled(
                format!("  {p}"),
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            ))
        }));
    }
    lines
}

fn render_section_lines<'a>(sections: &'a [InspectorSectionVm], theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    let mut jump_index = 1usize;
    for (section_idx, section) in sections.iter().enumerate() {
        if section_idx > 0 {
            lines.push(Line::from(""));
        }
        lines.push(section_heading_line(&section.heading, theme));
        let mut shortcut_rows = 0usize;
        for row in &section.rows {
            let shortcut = row_receives_shortcut(&section.heading, row)
                .then(|| {
                    shortcut_rows += 1;
                    if jump_index <= 9 {
                        let current = jump_index;
                        jump_index += 1;
                        Some(format!("[{current}] "))
                    } else {
                        None
                    }
                })
                .flatten();
            let disabled = section.heading == "ACTIONS" && row.label.contains("disabled");
            lines.push(inspector_section_row(
                row,
                shortcut.as_deref(),
                theme,
                disabled,
            ));
        }
        if section.heading == "RELATED" && shortcut_rows > 9 {
            lines.push(inspector_affordance_line(
                "[v] view all related",
                format!("{} total", shortcut_rows),
                theme,
            ));
        }
    }
    lines
}

fn section_heading_line<'a>(heading: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        heading,
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD),
    ))
}

fn inspector_affordance_line(
    label: impl Into<String>,
    secondary: impl Into<String>,
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "  ",
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            label.into(),
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", secondary.into()),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ),
    ])
}

fn inspector_section_row<'a>(
    row: &'a InspectorRow,
    shortcut: Option<&str>,
    theme: &Theme,
    dim: bool,
) -> Line<'a> {
    let mut spans = Vec::new();
    spans.push(Span::styled(
        "  ",
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.base_bg.color()),
    ));
    if let Some(shortcut) = shortcut {
        spans.push(Span::styled(
            shortcut.to_string(),
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ));
    }
    let row_fg = if dim {
        theme.footer.color()
    } else {
        theme.base_fg.color()
    };
    spans.push(Span::styled(
        format!("{:<13}", row.label),
        Style::default()
            .fg(theme.footer.color())
            .bg(theme.base_bg.color()),
    ));
    spans.push(Span::styled(
        row.value.clone(),
        Style::default().fg(row_fg).bg(theme.base_bg.color()),
    ));
    if let Some(secondary) = &row.secondary {
        spans.push(Span::styled(
            format!("  {secondary}"),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ));
    }
    Line::from(spans)
}

fn inspector_line<'a>(line: &'a str, theme: &Theme) -> Line<'a> {
    if let Some((label, value)) = line.split_once(':') {
        Line::from(vec![
            Span::styled(
                format!("{label:<10}"),
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            ),
            Span::styled(
                compact_text(value.trim(), 80),
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            ),
        ])
    } else {
        Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        ))
    }
}

fn render_logs(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    let p = Paragraph::new(view_model.inspector.lines.join("\n"))
        .wrap(Wrap { trim: false })
        .block(panel_block("Logs preview", theme, active))
        .style(
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color()),
        );
    frame.render_widget(p, area);
}
fn render_doctor_view(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
    selected_row: usize,
) {
    let title = format!(
        "Doctor — {} actionable, {} noise group(s)   [{}] Critical Warning Info All",
        view_model.doctor.actionable_count,
        view_model.doctor.noise_group_count,
        view_model.doctor.severity_filter.label()
    );
    if view_model.doctor.rows.is_empty() {
        render_empty_state(
            &title,
            "Everything's clean — no actionable warnings.",
            frame,
            area,
            theme,
            active,
        );
        return;
    }
    let rows = view_model.doctor.rows.iter().map(|row| {
        let severity_color = match row.severity_kind {
            DoctorSeverity::Error => theme.pip_error.color(),
            DoctorSeverity::Warning => theme.pip_warn.color(),
            DoctorSeverity::Info => theme.footer.color(),
        };
        let glyph = if row.tier == "noise" { "·" } else { "⚠" };
        Row::new(vec![
            Cell::from(Span::styled(
                format!("{glyph} {}", row.severity_kind.label()),
                Style::default()
                    .fg(severity_color)
                    .bg(theme.base_bg.color())
                    .add_modifier(Modifier::BOLD),
            )),
            Cell::from(Span::styled(
                compact_words(&row.check, 28),
                Style::default()
                    .fg(theme.accent.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                compact_text(&row.entity, 22),
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                compact_words(&row.details, 30),
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                compact_words(
                    &if row.is_group {
                        format!("{}  ×{}", row.suggested_action, row.count)
                    } else {
                        row.suggested_action.clone()
                    },
                    44,
                ),
                Style::default()
                    .fg(theme.info.color())
                    .bg(theme.base_bg.color()),
            )),
        ])
        .style(Style::default().bg(theme.base_bg.color()))
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(28),
            Constraint::Length(22),
            Constraint::Min(18),
            Constraint::Length(44),
        ],
    )
    .header(
        Row::new(["Severity", "Group", "Sample", "Code", "What to do"]).style(
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(panel_block(title, theme, active))
    .style(Style::default().bg(theme.base_bg.color()))
    .row_highlight_style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.selection.color())
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    if !view_model.doctor.rows.is_empty() {
        state.select(Some(
            selected_row.min(view_model.doctor.rows.len().saturating_sub(1)),
        ));
    }
    frame.render_stateful_widget(table, area, &mut state);
}
fn render_process_tree(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    let rows = view_model.process_tree.rows.iter().map(|r| {
        let marker = if r.expanded { "▾" } else { "▸" };
        let row_fg = if r.is_system {
            theme.system_noise.color()
        } else {
            theme.base_fg.color()
        };
        let runtime_fg = if r.is_system {
            theme.system_noise.color()
        } else {
            theme.info.color()
        };
        Row::new(vec![
            process_signal_cell(r, theme),
            Cell::from(Span::styled(
                format!("{}{} {}", "  ".repeat(r.depth), marker, r.label),
                Style::default().fg(row_fg).bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                r.runtime.clone(),
                Style::default().fg(runtime_fg).bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                r.workload.clone().unwrap_or_else(|| "-".into()),
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                r.warnings.join(","),
                Style::default()
                    .fg(theme.warning.color())
                    .bg(theme.base_bg.color()),
            )),
        ])
        .style(Style::default().bg(theme.base_bg.color()))
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(20),
        ],
    )
    .header(
        Row::new(["", "Process", "Runtime", "Workload", "Warnings"]).style(
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color()),
        ),
    )
    .block(panel_block(
        format!(
            "Process Tree — {} processes, {} roots",
            view_model.process_tree.rows.len(),
            view_model
                .process_tree
                .rows
                .iter()
                .filter(|row| row.depth == 0)
                .count()
        ),
        theme,
        active,
    ))
    .style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.base_bg.color()),
    )
    .row_highlight_style(
        Style::default()
            .fg(theme.base_fg.color())
            .bg(theme.selection.color())
            .add_modifier(Modifier::BOLD),
    );
    let mut state = TableState::default();
    if let Some(selected) = &view_model.process_tree.selected {
        if let Some(index) = view_model
            .process_tree
            .rows
            .iter()
            .position(|row| &row.key == selected)
        {
            state.select(Some(index));
        }
    }
    frame.render_stateful_widget(table, area, &mut state);
}
fn render_metrics(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Min(7),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(metrics_events_dropped_lines(&view_model.metrics, theme))
            .wrap(Wrap { trim: false })
            .block(panel_block("Events dropped", theme, false))
            .style(
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            ),
        chunks[0],
    );

    if view_model.metrics.adapters.is_empty()
        || view_model
            .metrics
            .adapters
            .iter()
            .all(|adapter| adapter.throughput == 0)
    {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "No events in last 60s — adapter is idle (this is normal).",
                    Style::default()
                        .fg(theme.ok.color())
                        .bg(theme.base_bg.color()),
                )),
                Line::from(Span::styled(
                    metric_caption("adapter_event_rate"),
                    Style::default()
                        .fg(theme.footer.color())
                        .bg(theme.base_bg.color()),
                )),
            ])
            .wrap(Wrap { trim: false })
            .block(panel_block("Adapter event rate", theme, active))
            .style(
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            ),
            chunks[1],
        );
    } else {
        frame.render_widget(
            Sparkline::default()
                .block(panel_block("Adapter event rate", theme, active))
                .data(&view_model.metrics.event_rate)
                .style(
                    Style::default()
                        .fg(theme.accent.color())
                        .bg(theme.base_bg.color()),
                ),
            chunks[1],
        );
    }

    let adapter_rows = view_model.metrics.adapters.iter().map(|adapter| {
        Row::new(vec![
            Cell::from(adapter.adapter.clone()),
            Cell::from(format!("{} / 60s", adapter.throughput)),
            Cell::from(adapter.drops.to_string()),
            Cell::from(
                adapter
                    .latency_ms
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "-".into()),
            ),
        ])
    });
    frame.render_widget(
        Table::new(
            adapter_rows,
            [
                Constraint::Min(12),
                Constraint::Length(14),
                Constraint::Length(8),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(["Adapter", "Events", "Drops", "Latency"]).style(
                Style::default()
                    .fg(theme.accent.color())
                    .bg(theme.base_bg.color()),
            ),
        )
        .block(panel_block("Adapter health", theme, false))
        .style(
            Style::default()
                .fg(theme.base_fg.color())
                .bg(theme.base_bg.color()),
        ),
        chunks[2],
    );

    let total = view_model.metrics.listeners_loopback + view_model.metrics.listeners_public;
    let bars = [
        Bar::default().label("Listeners".into()).value(total as u64),
        Bar::default()
            .label("Public".into())
            .value(view_model.metrics.listeners_public as u64),
        Bar::default()
            .label("Conflicts".into())
            .value(view_model.metrics.listeners_conflicts as u64),
        Bar::default()
            .label("Orphans".into())
            .value(view_model.metrics.listeners_orphans as u64),
    ];
    frame.render_widget(
        BarChart::default()
            .block(panel_block(
                format!(
                    "Listener exposure histogram — {}",
                    metric_caption("listener_histogram")
                ),
                theme,
                false,
            ))
            .direction(Direction::Horizontal)
            .data(BarGroup::default().bars(&bars))
            .bar_style(Style::default().fg(theme.ok.color())),
        chunks[3],
    );
}

fn metrics_events_dropped_lines(metrics: &MetricsVm, theme: &Theme) -> Vec<Line<'static>> {
    let dropped = metrics.events_dropped;
    let observed = metrics
        .adapters
        .iter()
        .map(|adapter| adapter.throughput)
        .sum::<u64>();
    let adapter_drops = metrics
        .adapters
        .iter()
        .map(|adapter| adapter.drops)
        .sum::<u64>();
    let dropped = dropped.max(adapter_drops);
    let mut lines = Vec::new();
    if dropped == 0 {
        lines.push(Line::from(Span::styled(
            "No events dropped in the observable window.",
            Style::default()
                .fg(theme.ok.color())
                .bg(theme.base_bg.color()),
        )));
    } else if observed > 0 {
        let denominator = observed + dropped;
        let pct = (dropped as f64 / denominator as f64) * 100.0;
        lines.push(Line::from(Span::styled(
            format!("{dropped} / {denominator} events dropped in last 60s = {pct:.1}%"),
            Style::default()
                .fg(theme.warning.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "drop counter unavailable in stateless run",
            Style::default()
                .fg(theme.warning.color())
                .bg(theme.base_bg.color())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("{dropped} event drop(s) were reported without a live 60s denominator."),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        )));
    }
    lines.push(Line::from(Span::styled(
        metric_caption("events_dropped"),
        Style::default()
            .fg(theme.footer.color())
            .bg(theme.base_bg.color()),
    )));
    lines
}

pub fn help_lines(keybindings: &ResolvedKeybindings) -> Vec<String> {
    let mut lines = keybindings
        .bindings
        .iter()
        .filter(|(_, b)| !b.is_empty())
        .map(|(a, b)| format!("{a}: {}", b.join(", ")))
        .collect::<Vec<_>>();
    lines.push("".into());
    lines.push("views: Overview · Listeners · Workloads · Processes · Doctor · Metrics".into());
    lines.push(
        "listeners chips: A all · P public · C conflicts · O orphans · U unowned · T tracked"
            .into(),
    );
    lines.push(
        "legacy views remain addressable via : public, : conflicts, : view all, or --view".into(),
    );
    lines
}

fn lazyadmin_state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("lazyadmin")
}

fn overview_seen_flag_path() -> PathBuf {
    lazyadmin_state_dir().join("seen-overview.flag")
}

fn overview_seen_flag_exists() -> bool {
    overview_seen_flag_path().exists()
}

fn mark_overview_seen() -> anyhow::Result<()> {
    let path = overview_seen_flag_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, b"seen\n")?;
    Ok(())
}

pub fn copy_diagnostic_fallback(
    markdown: &str,
    state_home: Option<&std::path::Path>,
) -> anyhow::Result<std::path::PathBuf> {
    let base = state_home
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("lazyadmin/copies");
    std::fs::create_dir_all(&base)?;
    let path = base.join(format!("{}.md", chrono::Utc::now().timestamp_millis()));
    std::fs::write(&path, markdown)?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyDiagnosticOutcome {
    Clipboard,
    File(PathBuf),
}

pub fn copy_diagnostic(markdown: &str) -> anyhow::Result<CopyDiagnosticOutcome> {
    copy_diagnostic_via_command(markdown)
        .map(|()| CopyDiagnosticOutcome::Clipboard)
        .or_else(|_| {
            copy_diagnostic_via_arboard(markdown).map(|()| CopyDiagnosticOutcome::Clipboard)
        })
        .or_else(|_| copy_diagnostic_fallback(markdown, None).map(CopyDiagnosticOutcome::File))
}

#[cfg(not(target_os = "linux"))]
fn copy_diagnostic_via_arboard(markdown: &str) -> anyhow::Result<()> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(markdown))
        .map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn copy_diagnostic_via_arboard(_markdown: &str) -> anyhow::Result<()> {
    anyhow::bail!("arboard clipboard disabled on linux TUI to avoid terminal stderr leaks")
}

fn copy_diagnostic_via_command(markdown: &str) -> anyhow::Result<()> {
    // Order matters: try the GUI-clipboard daemons in their native habitat
    // first, then fall back to less common ones. Each program's stdout/stderr
    // is sent to /dev/null so any "no display" / "selection lost" warnings
    // can't leak into the TUI's alternate screen (issue #7).
    for program in ["wl-copy", "xclip", "xsel", "pbcopy"] {
        let args: Vec<&str> = match program {
            "xclip" => vec!["-selection", "clipboard"],
            "xsel" => vec!["--clipboard", "--input"],
            _ => Vec::new(),
        };
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(ref mut child) = child {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(markdown.as_bytes())?;
            }
            let status = child.wait()?;
            if status.success() {
                return Ok(());
            }
        }
    }
    anyhow::bail!("no clipboard command succeeded")
}

pub fn open_url_for_row(row: &RowVm, allow_non_loopback: bool) -> anyhow::Result<String> {
    let port = row
        .port
        .ok_or_else(|| anyhow::anyhow!("open requires a TCP port"))?;
    let common_http_ports = [80, 443, 3000, 5000, 5173, 5174, 8000, 8080, 8443];
    if !common_http_ports.contains(&port) {
        anyhow::bail!("refusing to open uncommon HTTP port {port}");
    }
    let is_loopback = row.bind == "localhost"
        || row
            .bind
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false);
    if !is_loopback && !allow_non_loopback {
        anyhow::bail!("refusing to open non-loopback listener by default");
    }
    Ok(format!(
        "http://{}:{port}",
        if row.bind == "::1" {
            "[::1]"
        } else {
            row.bind.as_str()
        }
    ))
}

pub fn open_row_url(row: &RowVm, allow_non_loopback: bool) -> anyhow::Result<String> {
    let url = open_url_for_row(row, allow_non_loopback)?;
    open::that(&url)?;
    Ok(url)
}

pub fn headless_dump(
    snapshot: &Snapshot,
    width: u16,
    theme: Theme,
    keybindings: ResolvedKeybindings,
) -> HeadlessTuiDump {
    let vm = build_view_model(snapshot, width, false, "");
    HeadlessTuiDump {
        schema_version: "lazyadmin.tui.headless.v1".into(),
        layout: HeadlessLayout {
            width,
            mode: vm.layout,
        },
        panes: vec!["views".into(), "main".into(), "inspector".into()],
        theme: HeadlessTheme {
            name: theme.name.clone(),
            fallback_palette: theme.fallback_palette,
        },
        keybindings,
        view_model: vm,
    }
}

pub async fn run_tui(snapshot: Snapshot) -> Result<()> {
    run_tui_with_runtime(TuiRuntime::static_snapshot(snapshot)).await
}

pub async fn run_tui_with_runtime(mut runtime: TuiRuntime) -> Result<()> {
    install_panic_guard();
    let (w, _) = terminal::size().unwrap_or((80, 24));
    if w < 60 {
        println!(
            "lazyadmin TUI requires at least 60 columns. Try `lazyadmin ps`, `lazyadmin :PORT`, or widen the terminal."
        );
        return Ok(());
    }
    let _guard = TerminalGuard::enter()?;
    info!("tui.start");
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let initial_snapshot = runtime.initial_snapshot;
    let initial_view = runtime.initial_view.unwrap_or_default();
    let mut app = App {
        vm: build_view_model(&initial_snapshot, w, runtime.config.show_system, ""),
        snapshot: initial_snapshot,
        show_system: runtime.config.show_system,
        theme: runtime.theme,
        keybindings: runtime.keybindings,
        status: runtime.color_hint,
        allow_open_non_loopback: runtime.allow_open_non_loopback,
        config_reload: runtime.config_reload,
        active_view: initial_view,
        overview_hint_visible: initial_view == ViewKind::Overview && !overview_seen_flag_exists(),
        ..Default::default()
    };
    sync_row_selection(&mut app);
    let mut refresh_state =
        LiveRefreshState::new(runtime.config.event_debounce, runtime.config.max_redraw_hz);
    let started = Instant::now();
    loop {
        while let Some(rx) = runtime.discovery_events.as_mut() {
            match rx.try_recv() {
                Ok(event) => {
                    app.event_ring.record(&event, Instant::now());
                    refresh_state.on_event(&event, Instant::now());
                    app.vm.degraded = refresh_state.degraded.clone();
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    runtime.discovery_events = None;
                    break;
                }
            }
        }
        while let Some(rx) = runtime.snapshots.as_mut() {
            match rx.try_recv() {
                Ok(snapshot) => {
                    app.snapshot = snapshot;
                    rebuild_view_model(&mut app, w);
                    if let Some(degraded) = &refresh_state.degraded {
                        app.vm.degraded = Some(degraded.clone());
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    runtime.snapshots = None;
                    break;
                }
            }
        }
        if refresh_state.should_refresh(Instant::now()) {
            rebuild_view_model(&mut app, w);
            app.vm.degraded = refresh_state.degraded.clone().or(app.vm.degraded.clone());
        }
        let render_started = Instant::now();
        terminal.draw(|f| render_app(f, &app))?;
        debug!(
            elapsed_ms = render_started.elapsed().as_millis(),
            "tui.render"
        );
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                let input_started = Instant::now();
                handle_key(&mut app, key, w);
                debug!(
                    elapsed_ms = input_started.elapsed().as_millis(),
                    "tui.input"
                );
            }
        }
        if app.should_quit || started.elapsed() > Duration::from_secs(3600) {
            break;
        }
    }
    info!("tui.stop");
    Ok(())
}

fn rebuild_view_model(app: &mut App, width: u16) {
    app.vm = build_view_model_with_state(
        &app.snapshot,
        width,
        app.show_system,
        &app.query,
        app.selected_process.clone(),
        &app.collapsed_processes,
        Some(app.event_ring.metrics(Instant::now())),
        &app.doctor_toggled_groups,
        app.doctor_severity_filter,
    );
    sync_row_selection(app);
}

fn visible_row_indices(app: &App) -> Vec<usize> {
    app.vm
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| match app.active_view {
            ViewKind::Listeners | ViewKind::Public | ViewKind::Conflicts | ViewKind::Orphans => {
                row_matches_visible_listener_scope(
                    row,
                    app.active_view,
                    effective_listener_filter(app.active_view, app.listener_filter),
                    app.related_listener_filter.as_ref(),
                )
            }
            ViewKind::Ports => row.port.is_some(),
            ViewKind::TrackedRuns
            | ViewKind::Projects
            | ViewKind::Workloads
            | ViewKind::Managers
            | ViewKind::Processes
            | ViewKind::ProcessTree
            | ViewKind::Metrics
            | ViewKind::Logs
            | ViewKind::Doctor => false,
            _ => true,
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn sync_row_selection(app: &mut App) {
    if app.active_view == ViewKind::Overview {
        app.selected_row = app
            .selected_row
            .min(digest_target_count().saturating_sub(1));
        app.vm.inspector = InspectorVm {
            title: "Overview digest".into(),
            lines: vec![
                "Use ↑/↓ to choose a section.".into(),
                "Press Enter on [view all] to drill in.".into(),
            ],
            provenance: vec!["lazyadmin-runtime::view_model::Digest".into()],
            provenance_expanded: false,
            diagnostic_markdown: "# lazyadmin overview\nDigest projection from current snapshot\n"
                .into(),
            ..Default::default()
        };
        return;
    }
    if app.active_view == ViewKind::Doctor {
        if app.vm.doctor.rows.is_empty() {
            app.selected_row = 0;
            app.vm.inspector = plain_inspector(
                "No warning group",
                "Everything's clean — no actionable warnings.",
            );
            return;
        }
        app.selected_row = app
            .selected_row
            .min(app.vm.doctor.rows.len().saturating_sub(1));
        app.vm.inspector = inspector_for_doctor_row(&app.vm.doctor.rows[app.selected_row]);
        return;
    }
    let visible = visible_row_indices(app);
    if visible.is_empty() {
        app.selected_row = 0;
        if app.selected_process.is_none() {
            app.vm.inspector =
                plain_inspector("No selection", "No workloads/listeners discovered yet");
        }
        return;
    }
    if app.selected_row >= visible.len() {
        app.selected_row = visible.len().saturating_sub(1);
    }
    if app.selected_process.is_none() {
        if let Some(row) = visible
            .get(app.selected_row)
            .and_then(|idx| app.vm.rows.get(*idx))
        {
            app.vm.inspector = inspector_for_row(&app.snapshot, row);
        }
    }
}

fn digest_target_count() -> usize {
    4
}

fn digest_target_for_index(index: usize) -> ViewKind {
    match index {
        0 => ViewKind::Public,
        1 => ViewKind::Conflicts,
        2 => ViewKind::Projects,
        3 => ViewKind::Doctor,
        _ => ViewKind::Public,
    }
}

fn scroll_rows(app: &mut App, delta: isize) {
    let visible_len = if app.active_view == ViewKind::Overview {
        digest_target_count()
    } else if app.active_view == ViewKind::Doctor {
        app.vm.doctor.rows.len()
    } else {
        visible_row_indices(app).len()
    };
    if visible_len == 0 {
        app.selected_row = 0;
        sync_row_selection(app);
        return;
    }
    let max = visible_len.saturating_sub(1) as isize;
    app.selected_row = (app.selected_row as isize + delta).clamp(0, max) as usize;
    app.selected_process = None;
    sync_row_selection(app);
}

fn inspector_for_doctor_row(row: &DoctorRowVm) -> InspectorVm {
    let title = if row.is_group {
        format!("Warning group {}", row.code)
    } else {
        format!("Warning detail {}", row.code)
    };
    InspectorVm {
        title: title.clone(),
        lines: vec![
            format!("Tier: {}", row.tier),
            format!("Severity: {}", row.severity),
            format!("Code: {}", row.code),
            format!("Count: {}", row.count.max(1)),
            format!("Sample: {}", row.entity),
            format!("Action: {}", row.suggested_action),
        ],
        provenance: vec!["doctor_groups view-model".into()],
        provenance_expanded: false,
        diagnostic_markdown: format!(
            "# {title}\n\n- code: {}\n- action: {}\n",
            row.code, row.suggested_action
        ),
        ..Default::default()
    }
}

fn selected_row(app: &App) -> Option<&RowVm> {
    visible_row_indices(app)
        .get(app.selected_row)
        .and_then(|idx| app.vm.rows.get(*idx))
}

fn jump_to_inspector_target(app: &mut App, target: JumpTarget, width: u16) {
    match target {
        JumpTarget::Listener { id } => {
            let id = id.to_string();
            app.active_view = ViewKind::Listeners;
            app.listener_filter = ListenerFilter::All;
            app.related_listener_filter = None;
            app.selected_process = None;
            app.selected_row = 0;
            rebuild_view_model(app, width);
            if let Some(pos) = visible_row_indices(app)
                .iter()
                .position(|idx| app.vm.rows.get(*idx).is_some_and(|row| row.id == id))
            {
                app.selected_row = pos;
                sync_row_selection(app);
            }
            app.set_status(format!("jumped to listener {id}"));
        }
        JumpTarget::Process { key } => {
            app.active_view = ViewKind::ProcessTree;
            app.selected_process = Some(key.clone());
            app.selected_row = 0;
            rebuild_view_model(app, width);
            app.set_status(format!("jumped to pid {}", key.pid));
        }
        JumpTarget::Project { id } => {
            jump_to_lookup_only_target(app, width, ViewKind::Projects, "project", &id.to_string());
        }
        JumpTarget::Workload { id } => {
            jump_to_lookup_only_target(
                app,
                width,
                ViewKind::Workloads,
                "workload",
                &id.to_string(),
            );
        }
        JumpTarget::Manager { id } => {
            jump_to_lookup_only_target(app, width, ViewKind::Managers, "manager", &id.to_string());
        }
        JumpTarget::TrackedRun { id } => {
            jump_to_lookup_only_target(
                app,
                width,
                ViewKind::TrackedRuns,
                "tracked_run",
                &id.to_string(),
            );
        }
        JumpTarget::WarningGroup { code } => {
            app.active_view = ViewKind::Doctor;
            app.selected_process = None;
            app.selected_row = 0;
            rebuild_view_model(app, width);
            if let Some(pos) = app.vm.doctor.rows.iter().position(|row| row.code == code) {
                app.selected_row = pos;
                sync_row_selection(app);
            } else if let Some(view) = InspectorView::lookup(&app.snapshot, "warning_group", &code)
            {
                app.vm.inspector = inspector_vm_from_view(view);
            }
            app.set_status(format!("jumped to warning group {code}"));
        }
    }
}

fn view_all_related_listeners(app: &mut App, width: u16) {
    let ids = related_listener_ids(&app.vm.inspector.sections);
    if ids.len() <= 9 {
        app.set_status("no hidden related listeners to show");
        return;
    }
    let label = app.vm.inspector.title.clone();
    app.active_view = ViewKind::Listeners;
    app.listener_filter = ListenerFilter::All;
    app.related_listener_filter = Some(RelatedListenerFilter {
        ids: ids.into_iter().collect(),
        label: label.clone(),
    });
    app.selected_process = None;
    app.selected_row = 0;
    rebuild_view_model(app, width);
    info!(target = %label, "tui inspector view all related listeners");
    app.set_status(format!("viewing all related listeners for {label}"));
}

fn jump_to_lookup_only_target(app: &mut App, width: u16, view: ViewKind, kind: &str, id: &str) {
    app.active_view = view;
    app.selected_process = None;
    app.selected_row = 0;
    rebuild_view_model(app, width);
    if let Some(inspector) =
        InspectorView::lookup(&app.snapshot, kind, id).map(inspector_vm_from_view)
    {
        app.vm.inspector = inspector;
        app.set_status(format!("jumped to {kind} {id}"));
    } else {
        app.set_status(format!(
            "jump failed: {kind} {id} is no longer in the snapshot"
        ));
    }
}

fn action_target(row: Option<&RowVm>) -> String {
    row.map(|row| {
        let endpoint = row
            .port
            .map(|port| format!("{}:{port}", row.bind))
            .unwrap_or_else(|| row.bind.clone());
        let project = if row.project.trim().is_empty() || row.project == "-" {
            "no project".into()
        } else {
            row.project.clone()
        };
        format!(
            "{endpoint} owned by {} ({project}, {})",
            row.owner, row.runtime
        )
    })
    .unwrap_or_else(|| "no selected row".into())
}

fn start_confirmation(app: &mut App, command: Command, required: &str) {
    let target = action_target(selected_row(app));
    let command_preview = CommandDispatcher::plan_for_target(&command, &target);
    start_confirmation_with_preview(app, command, required, target, command_preview);
}

fn start_confirmation_with_preview(
    app: &mut App,
    command: Command,
    required: &str,
    target: String,
    command_preview: String,
) {
    app.confirmation = Some(Confirmation {
        command,
        typed: String::new(),
        required: required.into(),
        target: target.clone(),
        command_preview,
    });
    app.set_status(format!(
        "confirm {required} for {target}; type {required} then Enter, Esc cancels"
    ));
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InspectorAction {
    command: Command,
    required: String,
    target: String,
    command_preview: String,
    disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedInspectorAction {
    key_hint: String,
    action: InspectorAction,
}

fn handle_inspector_action_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(action) = inspector_action_for_key(&app.vm.inspector, key) else {
        return false;
    };
    if let Some(reason) = action.disabled_reason {
        app.set_status(format!("action disabled: {reason}"));
        return true;
    }
    info!(
        command = ?action.command,
        target = %action.target,
        preview = %action.command_preview,
        "tui inspector action confirmation opened"
    );
    start_confirmation_with_preview(
        app,
        action.command,
        &action.required,
        action.target,
        action.command_preview,
    );
    true
}

fn inspector_action_for_key(inspector: &InspectorVm, key: KeyEvent) -> Option<InspectorAction> {
    let pressed = key_event_to_spec(key)?;
    let mut action = inspector
        .sections
        .iter()
        .find(|section| section.heading == "ACTIONS")?
        .rows
        .iter()
        .filter_map(inspector_action_from_row)
        .find(|parsed| normalize_key_spec(&parsed.key_hint) == normalize_key_spec(&pressed))?
        .action;
    action.target = inspector.title.clone();
    Some(action)
}

fn inspector_action_from_row(row: &InspectorRow) -> Option<ParsedInspectorAction> {
    let (key_hint, rest) = row.label.strip_prefix('[')?.split_once("] ")?;
    let (verb, disabled_reason) = match rest.split_once(" — disabled (") {
        Some((verb, reason)) => (verb, Some(reason.trim_end_matches(')').to_string())),
        None => (rest, None),
    };
    let required = inspector_required_confirmation_text(verb);
    let command = inspector_command_for_verb(verb)?;
    Some(ParsedInspectorAction {
        key_hint: key_hint.to_string(),
        action: InspectorAction {
            command,
            required,
            target: verb.to_string(),
            command_preview: row.value.clone(),
            disabled_reason,
        },
    })
}

fn inspector_required_confirmation_text(verb: &str) -> String {
    if verb.starts_with("free") {
        "free".into()
    } else {
        verb.split_whitespace().next().unwrap_or(verb).to_string()
    }
}

fn inspector_command_for_verb(verb: &str) -> Option<Command> {
    match verb.split_whitespace().next()? {
        "free" => Some(Command::Free),
        "forget" => Some(Command::Run),
        "kill" => Some(Command::Kill),
        "logs" => Some(Command::Logs),
        "restart" => Some(Command::Restart),
        "stop" => Some(Command::Stop),
        _ => None,
    }
}

/// Confirmation modal owns ALL key input while it is open. This is intentional
/// for safety: it prevents global shortcuts (k, l, q, …) from running silently
/// while the user is mid-typing a destructive confirmation. Anything we don't
/// explicitly recognize is simply swallowed.
fn handle_confirmation_key(app: &mut App, key: KeyEvent) -> bool {
    let Some(mut confirmation) = app.confirmation.take() else {
        return false;
    };
    // Ctrl+C is the universal "get me out" reflex; treat it as Esc rather than
    // letting it land as `Char('c')` and silently grow the typed buffer.
    let is_ctrl_c =
        matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            app.set_status(format!(
                "cancelled {:?} for {}",
                confirmation.command, confirmation.target
            ));
        }
        KeyCode::Enter => {
            if confirmation.typed == confirmation.required {
                let target = confirmation.target.clone();
                let command = confirmation.command.clone();
                CommandDispatcher::execute(&command);
                app.set_status(format!(
                    "{}; awaiting action executor for {target}",
                    confirmation.command_preview
                ));
            } else {
                app.set_status(format!(
                    "confirmation failed for {}; type {} exactly",
                    confirmation.target, confirmation.required
                ));
                confirmation.typed.clear();
                app.confirmation = Some(confirmation);
            }
        }
        KeyCode::Backspace => {
            confirmation.typed.pop();
            app.confirmation = Some(confirmation);
        }
        KeyCode::Char(_) if is_ctrl_c => {
            app.set_status(format!(
                "cancelled {:?} for {}",
                confirmation.command, confirmation.target
            ));
        }
        KeyCode::Char(c) => {
            confirmation.typed.push(c);
            app.confirmation = Some(confirmation);
        }
        _ => {
            app.confirmation = Some(confirmation);
        }
    }
    true
}

fn navigable_views() -> &'static [ViewKind] {
    &[
        ViewKind::Overview,
        ViewKind::Listeners,
        ViewKind::Workloads,
        ViewKind::Processes,
        ViewKind::Doctor,
        ViewKind::Metrics,
    ]
}

fn active_view_index(view: ViewKind) -> usize {
    let rail_view = canonical_rail_view(view);
    navigable_views()
        .iter()
        .position(|candidate| *candidate == rail_view)
        .unwrap_or_default()
}

fn cycle_active_view(app: &mut App, delta: isize, width: u16) {
    let views = navigable_views();
    let len = views.len() as isize;
    let current = active_view_index(app.active_view) as isize;
    let next = (current + delta).rem_euclid(len) as usize;
    set_active_view(app, views[next], width);
}

fn set_active_view(app: &mut App, view: ViewKind, width: u16) {
    app.related_listener_filter = None;
    match view {
        ViewKind::Listeners => {
            app.selected_process = None;
            if !app.listeners_hint_seen {
                app.listeners_hint_visible = true;
                app.listeners_hint_seen = true;
            }
        }
        ViewKind::Public => {
            app.selected_process = None;
            app.listener_filter = ListenerFilter::Public;
        }
        ViewKind::Conflicts => {
            app.selected_process = None;
            app.listener_filter = ListenerFilter::Conflicts;
        }
        ViewKind::Orphans => {
            app.selected_process = None;
            app.listener_filter = ListenerFilter::Orphans;
        }
        ViewKind::ProcessTree | ViewKind::Processes => {
            if app.selected_process.is_none() {
                app.selected_process = app.vm.process_tree.rows.first().map(|row| row.key.clone());
            }
        }
        _ => {
            app.selected_process = None;
        }
    }
    app.active_view = view;
    app.selected_row = 0;
    rebuild_view_model(app, width);
    app.set_status(format!("view: {}", title_for_view(view)));
}

fn focus_pane(app: &mut App, pane: Pane) {
    app.pane = pane;
    app.set_status(format!(
        "focus: {}",
        match pane {
            Pane::Groups => "views",
            Pane::Rows => "main",
            Pane::Inspector => "inspector",
        }
    ));
}

fn cycle_pane(app: &mut App, delta: isize, width: u16) {
    if app.vm.layout != LayoutMode::ThreePane {
        cycle_active_view(app, delta, width);
        return;
    }
    let panes = [Pane::Groups, Pane::Rows, Pane::Inspector];
    let current = panes
        .iter()
        .position(|candidate| *candidate == app.pane)
        .unwrap_or(1) as isize;
    let next = (current + delta).rem_euclid(panes.len() as isize) as usize;
    focus_pane(app, panes[next]);
}

fn handle_key(app: &mut App, key: KeyEvent, width: u16) {
    if handle_confirmation_key(app, key) {
        return;
    }
    if app.active_view == ViewKind::Overview && app.overview_hint_visible {
        app.overview_hint_visible = false;
        if let Err(err) = mark_overview_seen() {
            app.set_status(format!("overview hint flag not saved: {err}"));
        }
    }
    if matches!(app.mode, InputMode::Filter) {
        match key.code {
            KeyCode::Esc => {
                app.query.clear();
                app.mode = InputMode::Normal;
                app.set_status("filter cleared");
                rebuild_view_model(app, width);
            }
            KeyCode::Enter => app.mode = InputMode::Normal,
            KeyCode::Backspace => {
                app.query.pop();
                rebuild_view_model(app, width);
            }
            KeyCode::Char(c) => {
                app.query.push(c);
                rebuild_view_model(app, width);
            }
            _ => {}
        }
        return;
    }
    if matches!(app.mode, InputMode::Palette) {
        match key.code {
            KeyCode::Esc => {
                app.query.clear();
                app.mode = InputMode::Normal;
            }
            KeyCode::Enter => {
                let command = app.query.clone();
                app.query.clear();
                app.mode = InputMode::Normal;
                run_palette_command(app, &command, width);
            }
            KeyCode::Backspace => {
                app.query.pop();
            }
            KeyCode::Char(c) => app.query.push(c),
            _ => {}
        }
        return;
    }
    if matches!(app.mode, InputMode::Help) && matches!(key.code, KeyCode::Esc | KeyCode::Char('?'))
    {
        app.mode = InputMode::Normal;
        return;
    }
    match key.code {
        KeyCode::Up => {
            if app.vm.layout == LayoutMode::ThreePane && app.pane == Pane::Groups {
                cycle_active_view(app, -1, width);
            } else if app.pane != Pane::Inspector {
                scroll_rows(app, -1);
            }
            return;
        }
        KeyCode::Down => {
            if app.vm.layout == LayoutMode::ThreePane && app.pane == Pane::Groups {
                cycle_active_view(app, 1, width);
            } else if app.pane != Pane::Inspector {
                scroll_rows(app, 1);
            }
            return;
        }
        KeyCode::PageUp => {
            scroll_rows(app, -10);
            return;
        }
        KeyCode::PageDown => {
            scroll_rows(app, 10);
            return;
        }
        KeyCode::Home => {
            app.selected_row = 0;
            app.selected_process = None;
            sync_row_selection(app);
            return;
        }
        KeyCode::End => {
            app.selected_row = if app.active_view == ViewKind::Overview {
                digest_target_count().saturating_sub(1)
            } else {
                visible_row_indices(app).len().saturating_sub(1)
            };
            app.selected_process = None;
            sync_row_selection(app);
            return;
        }
        _ => {}
    }
    if app.pane == Pane::Inspector
        && let KeyCode::Char(c) = key.code
        && let Some(index) = c.to_digit(10).and_then(|n| n.checked_sub(1))
        && let Some(target) = app.vm.inspector.jump_targets.get(index as usize).cloned()
    {
        jump_to_inspector_target(app, target, width);
        return;
    }
    if app.pane == Pane::Inspector && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')) {
        view_all_related_listeners(app, width);
        return;
    }
    if app.pane == Pane::Inspector && handle_inspector_action_key(app, key) {
        return;
    }
    if app.active_view == ViewKind::Overview
        && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V'))
    {
        set_active_view(app, ViewKind::Listeners, width);
        return;
    }
    if app.active_view == ViewKind::Listeners {
        match key.code {
            KeyCode::Char('a') | KeyCode::Char('A') => {
                set_listener_filter(app, ListenerFilter::All, width);
                return;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                set_listener_filter(app, ListenerFilter::Public, width);
                return;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                set_listener_filter(app, ListenerFilter::Conflicts, width);
                return;
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                set_listener_filter(app, ListenerFilter::Orphans, width);
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                set_listener_filter(app, ListenerFilter::Unowned, width);
                return;
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                set_listener_filter(app, ListenerFilter::Tracked, width);
                return;
            }
            _ => {}
        }
    }
    if app.active_view == ViewKind::Doctor {
        match key.code {
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.doctor_severity_filter = DoctorSeverityFilter::All;
                rebuild_view_model(app, width);
                return;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                app.doctor_severity_filter = DoctorSeverityFilter::Critical;
                rebuild_view_model(app, width);
                return;
            }
            KeyCode::Char('w') | KeyCode::Char('W') => {
                app.doctor_severity_filter = DoctorSeverityFilter::Warning;
                rebuild_view_model(app, width);
                return;
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                app.doctor_severity_filter = DoctorSeverityFilter::Info;
                rebuild_view_model(app, width);
                return;
            }
            _ => {}
        }
    }
    if let Some(cmd) = key_to_command_with_bindings(key, &app.keybindings) {
        match cmd {
            Command::Quit => app.should_quit = true,
            Command::ToggleSystem => {
                app.show_system = !app.show_system;
                rebuild_view_model(app, width);
            }
            Command::Filter => {
                if app.active_view == ViewKind::Overview {
                    set_active_view(app, ViewKind::Listeners, width);
                }
                app.mode = InputMode::Filter;
            }
            Command::Palette => {
                app.query.clear();
                app.mode = InputMode::Palette;
            }
            Command::Refresh => rebuild_view_model(app, width),
            Command::Inspect if app.active_view == ViewKind::Overview => match app.selected_row {
                0 => {
                    app.listener_filter = ListenerFilter::Public;
                    set_active_view(app, ViewKind::Listeners, width);
                }
                _ => {
                    let target = digest_target_for_index(app.selected_row);
                    set_active_view(app, target, width);
                }
            },
            Command::Inspect if app.active_view == ViewKind::Doctor => {
                toggle_selected_doctor_group(app, width);
            }
            Command::NextPane => cycle_pane(app, 1, width),
            Command::PrevPane => cycle_pane(app, -1, width),
            Command::Tree => {
                if matches!(app.active_view, ViewKind::ProcessTree | ViewKind::Processes) {
                    toggle_selected_process(app);
                    rebuild_view_model(app, width);
                } else {
                    set_active_view(app, ViewKind::ProcessTree, width);
                }
            }
            Command::Metrics => {
                set_active_view(app, ViewKind::Metrics, width);
            }
            Command::Logs => {
                set_active_view(app, ViewKind::Logs, width);
            }
            Command::Ports => {
                set_active_view(app, ViewKind::Ports, width);
            }
            Command::Help => {
                if app.active_view == ViewKind::Listeners {
                    app.listeners_hint_visible = false;
                }
                app.mode = InputMode::Help;
            }
            Command::CopyDiagnostic => match copy_diagnostic(&app.vm.inspector.diagnostic_markdown)
            {
                Ok(CopyDiagnosticOutcome::Clipboard) => {
                    app.set_status("diagnostic copied");
                }
                Ok(CopyDiagnosticOutcome::File(path)) => {
                    app.set_status(format!(
                        "clipboard unavailable; diagnostic written to {}",
                        path.display()
                    ));
                }
                Err(_) => app.set_status(
                    "clipboard unavailable; copy fallback failed, diagnostic remains in inspector",
                ),
            },
            Command::Open => match selected_row(app) {
                Some(row) => match open_row_url(row, app.allow_open_non_loopback) {
                    Ok(url) => app.set_status(format!("opened {url}")),
                    Err(err) => app.set_status(format!("open failed: {err}")),
                },
                None => app.set_status("open failed: no selected listener"),
            },
            Command::Kill => match selected_row(app) {
                Some(_) => start_confirmation(app, cmd, "kill"),
                None => app.set_status("kill failed: no selected listener"),
            },
            Command::Restart | Command::Stop | Command::Free | Command::Run => {
                match selected_row(app) {
                    Some(row) => {
                        app.set_status(CommandDispatcher::plan(&cmd, Some(row)));
                    }
                    None => {
                        app.set_status(format!("{cmd:?} failed: no selected listener", cmd = cmd));
                    }
                }
            }
            Command::Edit => match selected_row(app) {
                Some(row) => {
                    app.set_status(format!(
                        "edit not implemented for {}",
                        action_target(Some(row))
                    ));
                }
                None => {
                    app.set_status("edit failed: no selected listener");
                }
            },
            _ => CommandDispatcher::execute(&cmd),
        }
    }
}

fn set_listener_filter(app: &mut App, filter: ListenerFilter, width: u16) {
    app.listener_filter = filter;
    app.related_listener_filter = None;
    app.listeners_hint_visible = false;
    rebuild_view_model(app, width);
    app.set_status(format!("listeners filter: {}", filter.label()));
}

fn toggle_selected_doctor_group(app: &mut App, width: u16) {
    let Some(row) = app.vm.doctor.rows.get(app.selected_row) else {
        return;
    };
    if !row.is_group {
        app.set_status("select a warning group row to expand or collapse");
        return;
    }
    let severity = match row.severity_kind {
        DoctorSeverity::Error => WarningSeverity::Error,
        DoctorSeverity::Warning => WarningSeverity::Warning,
        DoctorSeverity::Info => WarningSeverity::Info,
    };
    let key = doctor_group_key(&row.code, &severity);
    if !app.doctor_toggled_groups.insert(key.clone()) {
        app.doctor_toggled_groups.remove(&key);
    }
    rebuild_view_model(app, width);
}

fn toggle_selected_process(app: &mut App) {
    let selected = app
        .selected_process
        .clone()
        .or_else(|| app.vm.process_tree.rows.first().map(|row| row.key.clone()));
    if let Some(key) = selected {
        if !app.collapsed_processes.insert(key.clone()) {
            app.collapsed_processes.remove(&key);
        }
        app.selected_process = Some(key);
    }
}

fn run_palette_command(app: &mut App, command: &str, width: u16) {
    match command.trim() {
        "reload" => {
            if let Some(reload) = app.config_reload.as_mut() {
                match reload() {
                    Ok((theme, keybindings)) => {
                        app.theme = theme;
                        app.keybindings = keybindings;
                        app.set_status("config reloaded");
                    }
                    Err(err) => app.set_status(format!("reload failed: {err}")),
                }
            } else {
                app.set_status("reload unavailable in this runtime");
            }
        }
        "overview" | "digest" => set_active_view(app, ViewKind::Overview, width),
        "listeners" => set_active_view(app, ViewKind::Listeners, width),
        "view all" | "everything" | "all" => set_active_view(app, ViewKind::Everything, width),
        "workloads" => set_active_view(app, ViewKind::Workloads, width),
        "processes" => set_active_view(app, ViewKind::Processes, width),
        "public" => set_active_view(app, ViewKind::Public, width),
        "conflicts" => set_active_view(app, ViewKind::Conflicts, width),
        "projects" => set_active_view(app, ViewKind::Projects, width),
        "doctor" | "warnings" => set_active_view(app, ViewKind::Doctor, width),
        "process-tree" | "show-process-tree" => {
            app.active_view = ViewKind::ProcessTree;
            rebuild_view_model(app, width);
        }
        "metrics" => {
            app.active_view = ViewKind::Metrics;
            rebuild_view_model(app, width);
        }
        value if value.starts_with("theme ") => {
            let name = value.trim_start_matches("theme ").trim();
            match Theme::load(Some(name), None) {
                Ok(theme) => {
                    app.theme = theme;
                    app.set_status(format!("theme {name} applied"));
                }
                Err(err) => app.set_status(format!("theme failed: {err}")),
            }
        }
        "" => {}
        other => app.set_status(format!("unknown command: {other}")),
    }
}
fn render_app(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();
    let ctx = RenderContext {
        view: app.active_view,
        active_pane: app.pane,
        keybindings: Some(&app.keybindings),
        selected_row: app.selected_row,
        overview_hint_visible: app.overview_hint_visible,
        listener_filter: app.listener_filter,
        listeners_hint_visible: app.listeners_hint_visible,
        related_listener_filter: app.related_listener_filter.as_ref(),
    };
    render_view_kind(&app.vm, f, area, &app.theme, ctx);
    let footer = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let input_footer = match app.mode {
        InputMode::Filter => Some(format!("Filter: {}  (Enter apply, Esc clear)", app.query)),
        InputMode::Palette => Some(format!("Command: {}  (Enter run, Esc cancel)", app.query)),
        _ => None,
    };
    if let Some(input_footer) = input_footer {
        f.render_widget(
            Paragraph::new(pad_to_width(
                Line::from(Span::styled(
                    input_footer,
                    Style::default()
                        .fg(app.theme.accent.color())
                        .bg(app.theme.base_bg.color()),
                )),
                footer.width,
            ))
            .style(Style::default().bg(app.theme.base_bg.color())),
            footer,
        );
    } else if app.confirmation.is_none() {
        render_toast_overlay(f, app, area);
    }
    if matches!(app.mode, InputMode::Help) {
        let area = centered_rect(70, 70, f.area());
        let help = Paragraph::new(help_lines(&app.keybindings).join("\n"))
            .wrap(Wrap { trim: false })
            .block(panel_block("Help — active keybindings", &app.theme, true))
            .style(
                Style::default()
                    .fg(app.theme.base_fg.color())
                    .bg(app.theme.base_bg.color()),
            );
        f.render_widget(Clear, area);
        f.render_widget(help, area);
    }
    if matches!(app.mode, InputMode::Palette) {
        let area = centered_rect(50, 45, f.area());
        let entries = palette_entries(&app.query);
        let mut lines = vec![format!("Command: {}", app.query), "".into()];
        lines.extend(entries.iter().take(12).map(|entry| format!("  {entry}")));
        if entries.len() > 12 {
            lines.push(format!("  … {} more", entries.len() - 12));
        }
        let palette = Paragraph::new(lines.join("\n"))
            .wrap(Wrap { trim: false })
            .block(panel_block("Command palette", &app.theme, true))
            .style(
                Style::default()
                    .fg(app.theme.base_fg.color())
                    .bg(app.theme.base_bg.color()),
            );
        f.render_widget(Clear, area);
        f.render_widget(palette, area);
    }
    if let Some(confirmation) = &app.confirmation {
        let area = centered_rect(62, 38, f.area());
        let modal_title = format!(
            "Confirm action — {}",
            compact_text(&confirmation.command_preview, 42)
        );
        let body = vec![
            Line::from(vec![
                Span::styled("Preview: ", Style::default().fg(app.theme.footer.color())),
                Span::raw(confirmation.command_preview.clone()),
            ]),
            Line::from(vec![
                Span::styled("Action: ", Style::default().fg(app.theme.footer.color())),
                Span::raw(format!("{:?}", confirmation.command)),
            ]),
            Line::from(vec![
                Span::styled("Target: ", Style::default().fg(app.theme.footer.color())),
                Span::raw(confirmation.target.clone()),
            ]),
            Line::from(""),
            Line::from(format!(
                "Type '{}' then Enter to continue.",
                confirmation.required
            )),
            Line::from("Esc cancels. Other global keys are disabled while confirming."),
            Line::from(""),
            Line::from(vec![
                Span::styled("Input: ", Style::default().fg(app.theme.footer.color())),
                Span::styled(
                    confirmation.typed.clone(),
                    Style::default()
                        .fg(app.theme.accent.color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        let prompt = Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .block(panel_block(modal_title, &app.theme, true))
            .style(
                Style::default()
                    .fg(app.theme.base_fg.color())
                    .bg(app.theme.base_bg.color()),
            );
        f.render_widget(Clear, area);
        f.render_widget(prompt, area);
    }
}

fn active_toast_message(app: &App, now: Instant) -> Option<String> {
    let input_active = matches!(app.mode, InputMode::Filter | InputMode::Palette);
    app.toasts
        .iter()
        .rev()
        .find(|toast| {
            if input_active {
                return true;
            }
            toast
                .created_at
                .is_none_or(|created_at| now.duration_since(created_at) <= toast.ttl)
        })
        .map(|toast| toast.message.clone())
        .or_else(|| app.status.clone())
}

fn render_toast_overlay(f: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let Some(message) = active_toast_message(app, Instant::now()) else {
        return;
    };
    if message.is_empty() || area.height < 2 {
        return;
    }
    let toast_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(2),
        width: area.width,
        height: 1,
    };
    f.render_widget(Clear, toast_area);
    f.render_widget(
        Paragraph::new(pad_to_width(
            Line::from(Span::styled(
                message,
                Style::default()
                    .fg(app.theme.base_fg.color())
                    .bg(app.theme.selection.color()),
            )),
            toast_area.width,
        )),
        toast_area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[allow(dead_code)]
fn render_app_legacy(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();
    let chunks = match app.vm.layout {
        LayoutMode::ThreePane => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(NAV_PANE_WIDTH),
                Constraint::Min(MAIN_PANE_MIN_WIDTH),
                Constraint::Length(INSPECTOR_PANE_WIDTH),
            ])
            .split(area),
        LayoutMode::InspectorTab | LayoutMode::SinglePane | LayoutMode::Refuse => Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(area),
    };
    let groups = List::new(
        app.vm
            .groups
            .iter()
            .map(|g| ListItem::new(g.clone()))
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title("Groups / Filters")
            .borders(Borders::ALL),
    );
    let rows = List::new(
        app.vm
            .rows
            .iter()
            .map(|r| {
                ListItem::new(format!(
                    "{:>5} {:<15} {:<16} {:<10} {}",
                    r.port.unwrap_or_default(),
                    r.bind,
                    r.owner,
                    r.runtime,
                    r.badges.join(" ")
                ))
            })
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title(if app.vm.hidden_system_count > 0 {
                format!(
                    "Workloads / Listeners — {} system listeners hidden; press S",
                    app.vm.hidden_system_count
                )
            } else {
                "Workloads / Listeners".into()
            })
            .borders(Borders::ALL),
    );
    if app.vm.layout == LayoutMode::ThreePane {
        f.render_widget(groups, chunks[0]);
        f.render_widget(rows, chunks[1]);
        let insp = Paragraph::new(
            app.vm
                .inspector
                .lines
                .iter()
                .map(|l| Line::from(l.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::default()
                .title(app.vm.inspector.title.clone())
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::Gray));
        f.render_widget(insp, chunks[2]);
    } else {
        f.render_widget(rows, chunks[0]);
    }
}

pub async fn run_default() -> Result<()> {
    run_tui(build_empty_snapshot()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use lazyadmin_core::model::{
        AddressFamily, Confidence, DualStackState, Listener, ListenerId, ListenerState, Project,
        ProjectId, Protocol, RunId, TrackedRun, WarningSeverity, WorkloadState,
    };
    use ratatui::backend::TestBackend;
    #[test]
    fn keymap_covers_plan_keys() {
        assert_eq!(
            key_to_command(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
            Some(Command::Filter)
        );
        assert_eq!(
            key_to_command(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE)),
            Some(Command::Palette)
        );
        assert_eq!(
            key_to_command(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Command::Quit)
        );
        assert_eq!(
            key_to_command(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE)),
            Some(Command::ToggleSystem)
        );
    }
    #[test]
    fn palette_fuzzy() {
        assert!(palette_entries("free").contains(&"free-port"));
    }
    #[test]
    fn action_confirmation_requires_text() {
        let mut app = app_with_listener(8080);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.confirmation.as_ref().unwrap().required, "kill");
        assert!(app.confirmation.as_ref().unwrap().target.contains("8080"));
    }

    /// Mutating action shortcuts must refuse rather than silently arm a
    /// confirmation/dry-run against "no selected row".
    #[test]
    fn mutating_actions_refuse_when_no_row_is_selected() {
        for (key, expected_substr) in [
            ('k', "kill failed"),
            ('r', "Restart failed"),
            ('s', "Stop failed"),
            ('f', "Free failed"),
            ('R', "Run failed"),
            ('e', "edit failed"),
        ] {
            let mut app = App::default();
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                120,
            );
            assert!(
                app.confirmation.is_none(),
                "{key} must not arm a confirmation against no row"
            );
            let status = app.status.as_deref().unwrap_or_default();
            assert!(
                status.contains(expected_substr),
                "{key} status missing {expected_substr}: {status}"
            );
        }
    }
    #[test]
    fn view_model_widths() {
        let s = build_empty_snapshot();
        assert_eq!(
            build_view_model(&s, 120, false, "").layout,
            LayoutMode::ThreePane
        );
        assert_eq!(
            build_view_model(&s, 90, false, "").layout,
            LayoutMode::InspectorTab
        );
        assert_eq!(
            build_view_model(&s, 70, false, "").layout,
            LayoutMode::SinglePane
        );
        assert_eq!(
            build_view_model(&s, 50, false, "").layout,
            LayoutMode::Refuse
        );
    }

    #[test]
    fn default_view_is_overview() {
        assert_eq!(ViewKind::default(), ViewKind::Overview);
        assert_eq!(App::default().active_view, ViewKind::Overview);
    }

    #[test]
    fn digest_renders_at_120_90_70_cols() {
        let snapshot = build_empty_snapshot();
        for width in [120, 90, 70] {
            let vm = build_view_model(&snapshot, width, false, "");
            let backend = TestBackend::new(width, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| {
                    render_view_kind(
                        &vm,
                        f,
                        f.area(),
                        &Theme::default_dark(),
                        RenderContext {
                            view: ViewKind::Overview,
                            active_pane: Pane::Rows,
                            keybindings: None,
                            selected_row: 0,
                            overview_hint_visible: false,
                            listener_filter: ListenerFilter::All,
                            listeners_hint_visible: false,
                            related_listener_filter: None,
                        },
                    )
                })
                .unwrap();
            let text = format!("{:?}", terminal.backend().buffer());
            assert!(
                text.contains("EXPOSED"),
                "width {width} missing exposed section: {text}"
            );
            assert!(
                text.contains("TRIAGE"),
                "width {width} missing triage section: {text}"
            );
        }
    }

    #[test]
    fn digest_empty_state_strings_present() {
        let snapshot = build_empty_snapshot();
        let vm = build_view_model(&snapshot, 120, false, "");
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Overview,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains(lazyadmin_runtime::view_model::digest::EMPTY_EXPOSED));
        assert!(text.contains(lazyadmin_runtime::view_model::digest::EMPTY_CONFLICTS));
        assert!(text.contains(lazyadmin_runtime::view_model::digest::EMPTY_PROJECTS));
    }

    #[test]
    fn digest_drilldown_navigates_to_listeners_with_public_chip() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Overview,
            selected_row: 0,
            ..Default::default()
        };
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Listeners);
        assert_eq!(app.listener_filter, ListenerFilter::Public);
    }

    #[test]
    fn digest_refuse_mode_collapses_to_section_summary() {
        let snapshot = build_empty_snapshot();
        let vm = build_view_model(&snapshot, 50, false, "");
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Overview,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("EXPOSED 0"));
        assert!(text.contains("TRIAGE"));
        assert!(text.contains("actionable"));
    }

    #[test]
    fn render_views_golden_widths() {
        let s = build_empty_snapshot();
        for width in [120, 90, 70, 50] {
            let vm = build_view_model(&s, width, false, "");
            let backend = TestBackend::new(width, 20);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render(&vm, f, f.area(), &Theme::default_dark()))
                .unwrap();
            let text = format!("{:?}", terminal.backend().buffer());
            if width < 60 {
                assert!(text.contains("EXPOSED"));
            } else {
                assert!(text.contains("Overview") || text.contains("Views"));
            }
        }
    }

    #[test]
    fn render_views_hidden_count_footer_appears() {
        let mut vm = build_view_model(&build_empty_snapshot(), 120, false, "");
        vm.hidden_system_count = 12;
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(&vm, f, f.area(), &Theme::default_dark()))
            .unwrap();
        assert!(format!("{:?}", terminal.backend().buffer()).contains("hidden"));
    }

    /// Every view kind must produce a refusal message that names at least one
    /// matching CLI command. Issue #6 acceptance criterion 3.
    #[test]
    fn narrow_refusal_lists_view_specific_cli_hint() {
        for view in [
            ViewKind::Overview,
            ViewKind::Listeners,
            ViewKind::Workloads,
            ViewKind::Processes,
            ViewKind::Everything,
            ViewKind::Ports,
            ViewKind::Public,
            ViewKind::Conflicts,
            ViewKind::Orphans,
            ViewKind::TrackedRuns,
            ViewKind::Projects,
            ViewKind::Logs,
            ViewKind::Doctor,
            ViewKind::ProcessTree,
            ViewKind::Metrics,
            ViewKind::Managers,
        ] {
            let message = narrow_refusal_message(view);
            assert!(
                message.contains("60+ columns"),
                "{view:?} message lost size hint: {message}"
            );
            let hints = cli_hints_for_view(view);
            assert!(!hints.is_empty(), "{view:?} has no CLI hint configured");
            for hint in hints {
                assert!(
                    message.contains(hint),
                    "{view:?} message missing hint {hint}: {message}"
                );
            }
            // Title of the active view should appear so the user knows which
            // view was refused.
            assert!(
                message.contains(title_for_view(view)),
                "{view:?} message missing view title: {message}"
            );
        }
    }

    #[test]
    fn narrow_render_refuses_even_if_view_model_was_built_wide() {
        let vm = build_view_model(&build_empty_snapshot(), 120, false, "");
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Projects,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("60+ columns"));
        // Refusal message references the active view's CLI command but must
        // NOT have rendered the Projects panel/table itself.
        assert!(text.contains("lazyadmin projects"));
        assert!(!text.contains("No projects"));
    }

    #[test]
    fn special_views_render_projections_or_empty_states_not_listener_table() {
        let mut snap = build_empty_snapshot();
        snap.projects.push(Project {
            id: ProjectId::new("p1"),
            root: std::path::PathBuf::from("/tmp/demo"),
            name: "demo".into(),
            markers: vec![],
            git_remote: None,
            package_manager: Some("npm".into()),
            dev_commands: vec![],
            provenance: vec![],
        });
        snap.tracked_runs.push(TrackedRun {
            id: RunId::new("run1"),
            tag: Some("api".into()),
            command: vec!["npm".into(), "run".into(), "dev".into()],
            cwd: None,
            state: WorkloadState::Running,
            started_at: None,
            provenance: vec![],
        });
        // Multi-owner listener mirrors the CLI `conflicts` shape: a real socket
        // contended for, not a free-floating warning.
        let proc = process(7777, None, 1);
        let proc_key = proc.key.clone();
        snap.processes.push(proc);
        let mut conflicting = listener_for_process(proc_key, 8081);
        conflicting.owners.push(EntityRef::Process(ProcessKey {
            pid: 7778,
            boot_id: "boot".into(),
            start_time_ticks: 2,
        }));
        snap.listeners.push(conflicting);
        let vm = build_view_model(&snap, 120, false, "");
        // The conflicts row's full endpoint may be column-truncated at 120 cols
        // when the inspector pane is showing. Assert against the view-model so
        // we exercise the projection shape independently, then assert a
        // truncation-safe prefix in the rendered buffer below.
        assert_eq!(vm.conflicts.len(), 1);
        assert_eq!(vm.conflicts[0].name, "127.0.0.1:8081");
        for (view, expected) in [
            (ViewKind::Conflicts, "127.0.0.1"),
            (ViewKind::Orphans, "No orphan"),
            (ViewKind::TrackedRuns, "npm run dev"),
            (ViewKind::Projects, "/tmp/demo"),
        ] {
            let backend = TestBackend::new(120, 20);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| {
                    render_view_kind(
                        &vm,
                        f,
                        f.area(),
                        &Theme::default_dark(),
                        RenderContext {
                            view,
                            active_pane: Pane::Rows,
                            keybindings: None,
                            selected_row: 0,
                            overview_hint_visible: false,
                            listener_filter: ListenerFilter::All,
                            listeners_hint_visible: false,
                            related_listener_filter: None,
                        },
                    )
                })
                .unwrap();
            let text = format!("{:?}", terminal.backend().buffer());
            assert!(
                text.contains(expected),
                "{view:?} missing {expected}: {text}"
            );
            // Each special view must render its own panel title, not the
            // Everything table's header. The Everything header is "Listeners";
            // the special views use their own panel titles.
            let title = title_for_view(view);
            assert!(
                text.contains(title),
                "{view:?} missing its panel title {title}: {text}"
            );
        }
    }

    /// Conflicts/Orphans projections must align with the CLI shape. The CLI's
    /// `conflicts` view (lazyadmin-cli/src/main.rs) selects listeners that are
    /// referenced by a CONFLICT warning OR have more than one owner; the TUI's
    /// `conflict_rows` must agree on which listener IDs participate.
    fn cli_conflict_listener_ids(snapshot: &Snapshot) -> std::collections::HashSet<String> {
        use std::collections::HashSet;
        let conflict_ids: HashSet<_> = snapshot
            .warnings
            .iter()
            .filter(|w| w.code == "CONFLICT")
            .filter_map(|w| match &w.entity {
                Some(EntityRef::Listener(id)) => Some(id.clone()),
                _ => None,
            })
            .collect();
        snapshot
            .listeners
            .iter()
            .filter(|l| conflict_ids.contains(&l.id) || l.owners.len() > 1)
            .map(|l| l.id.to_string())
            .collect()
    }

    #[test]
    fn conflict_and_orphan_projections_match_cli_listener_shape() {
        let mut snap = build_empty_snapshot();
        // Conflict by multi-owner: two owners, no warning.
        let p1 = process(1001, None, 1);
        let p1_key = p1.key.clone();
        snap.processes.push(p1);
        let mut multi_owner = listener_for_process(p1_key, 8001);
        multi_owner.owners.push(EntityRef::Process(ProcessKey {
            pid: 1002,
            boot_id: "boot".into(),
            start_time_ticks: 2,
        }));
        let multi_owner_id = multi_owner.id.to_string();
        snap.listeners.push(multi_owner);
        // Conflict by warning entity: single-owner listener flagged via warning.
        let p3 = process(1003, None, 3);
        let p3_key = p3.key.clone();
        snap.processes.push(p3);
        let warned = listener_for_process(p3_key, 8002);
        let warned_id = warned.id.clone();
        let warned_id_str = warned_id.to_string();
        snap.listeners.push(warned);
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "CONFLICT".into(),
            message: "duplicate bind".into(),
            entity: Some(EntityRef::Listener(warned_id)),
            provenance: vec![],
        });
        // Orphan: listener with no owners at all.
        let now = chrono::Utc::now();
        let orphan = Listener {
            id: ListenerId::new("tcp:0.0.0.0:9999".to_string()),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("0.0.0.0".into()),
            port: Some(9999),
            path: None,
            state: ListenerState::Listen,
            netns: "default".into(),
            socket_inode: None,
            exposure: Exposure::LanOrPublic,
            owners: vec![],
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: now,
            last_seen: now,
            dual_stack_state: DualStackState::Unknown,
        };
        snap.listeners.push(orphan);
        // Sanity: the TUI projections must surface the same listener IDs the
        // CLI would project, no more, no less.
        let expected_conflict_ids = cli_conflict_listener_ids(&snap);
        assert_eq!(expected_conflict_ids.len(), 2);
        assert!(expected_conflict_ids.contains(&multi_owner_id));
        assert!(expected_conflict_ids.contains(&warned_id_str));

        let vm = build_view_model(&snap, 120, false, "");
        // Conflicts: each row's `name` is `bind:port`, derived from the listener.
        let conflict_endpoints: std::collections::HashSet<_> =
            vm.conflicts.iter().map(|r| r.name.clone()).collect();
        assert_eq!(conflict_endpoints.len(), 2);
        assert!(conflict_endpoints.contains("127.0.0.1:8001"));
        assert!(conflict_endpoints.contains("127.0.0.1:8002"));

        // Orphans: only the unowned listener appears.
        assert_eq!(vm.orphans.len(), 1);
        assert_eq!(vm.orphans[0].name, "0.0.0.0:9999");
    }

    #[test]
    fn listener_chip_predicates_match_row_flags() {
        let mut snap = build_empty_snapshot();
        let mut public_proc = process(2001, None, 1);
        public_proc.lazyadmin_run_id = Some(RunId::new("tracked"));
        let public_key = public_proc.key.clone();
        snap.processes.push(public_proc);
        let mut public = listener_for_process(public_key, 8080);
        public.exposure = Exposure::Public;
        public.bind_addr = Some("0.0.0.0".into());
        snap.listeners.push(public);

        let conflict_proc = process(2002, None, 2);
        let conflict_key = conflict_proc.key.clone();
        snap.processes.push(conflict_proc);
        let mut conflict = listener_for_process(conflict_key, 8081);
        conflict.owners.push(EntityRef::Process(ProcessKey {
            pid: 2003,
            boot_id: "boot".into(),
            start_time_ticks: 3,
        }));
        snap.listeners.push(conflict);

        let now = chrono::Utc::now();
        snap.listeners.push(Listener {
            id: ListenerId::new("tcp:127.0.0.1:8082"),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("127.0.0.1".into()),
            port: Some(8082),
            path: None,
            state: ListenerState::Listen,
            netns: "default".into(),
            socket_inode: None,
            exposure: Exposure::Loopback,
            owners: vec![],
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: now,
            last_seen: now,
            dual_stack_state: DualStackState::NotApplicable,
        });

        let vm = build_view_model(&snap, 120, false, "");
        assert_eq!(
            vm.rows
                .iter()
                .filter(|row| row_matches_listener_filter(row, ListenerFilter::All))
                .count(),
            3
        );
        assert_eq!(
            vm.rows
                .iter()
                .filter(|row| row_matches_listener_filter(row, ListenerFilter::Public))
                .count(),
            1
        );
        assert_eq!(
            vm.rows
                .iter()
                .filter(|row| row_matches_listener_filter(row, ListenerFilter::Conflicts))
                .count(),
            1
        );
        assert_eq!(
            vm.rows
                .iter()
                .filter(|row| row_matches_listener_filter(row, ListenerFilter::Orphans))
                .count(),
            1
        );
        assert_eq!(
            vm.rows
                .iter()
                .filter(|row| row_matches_listener_filter(row, ListenerFilter::Tracked))
                .count(),
            1
        );
    }

    #[test]
    fn listener_chip_predicates_match_busy_fixture_legacy_views() {
        let snap: Snapshot =
            serde_json::from_str(include_str!("../../../testdata/snapshots/busy.json"))
                .expect("busy snapshot fixture parses");
        let vm = build_view_model(&snap, 120, false, "");
        let public_rows = vm
            .rows
            .iter()
            .filter(|row| row_matches_listener_filter(row, ListenerFilter::Public))
            .count();
        let public_legacy = vm
            .rows
            .iter()
            .filter(|row| row_matches_view_filter(row, ViewKind::Public, ListenerFilter::Public))
            .count();
        let conflict_rows = vm
            .rows
            .iter()
            .filter(|row| row_matches_listener_filter(row, ListenerFilter::Conflicts))
            .count();
        let orphan_rows = vm
            .rows
            .iter()
            .filter(|row| row_matches_listener_filter(row, ListenerFilter::Orphans))
            .count();
        assert_eq!(public_rows, public_legacy);
        assert_eq!(conflict_rows, vm.conflicts.len());
        assert_eq!(orphan_rows, vm.orphans.len());
    }

    #[test]
    fn risk_glyphs_present_without_color() {
        let mut snap = build_empty_snapshot();

        let public_proc = process(4001, None, 1);
        let public_key = public_proc.key.clone();
        snap.processes.push(public_proc);
        let mut public = listener_for_process(public_key, 8080);
        public.exposure = Exposure::Public;
        public.bind_addr = Some("0.0.0.0".into());
        snap.listeners.push(public);

        let lan_proc = process(4002, None, 2);
        let lan_key = lan_proc.key.clone();
        snap.processes.push(lan_proc);
        let mut lan = listener_for_process(lan_key, 8081);
        lan.exposure = Exposure::LanOrPublic;
        lan.bind_addr = Some("0.0.0.0".into());
        snap.listeners.push(lan);

        let conflict_proc = process(4003, None, 3);
        let conflict_key = conflict_proc.key.clone();
        snap.processes.push(conflict_proc);
        let mut conflict = listener_for_process(conflict_key, 8082);
        conflict.owners.push(EntityRef::Process(ProcessKey {
            pid: 4004,
            boot_id: "boot".into(),
            start_time_ticks: 4,
        }));
        snap.listeners.push(conflict);

        let mut tracked_proc = process(4005, None, 5);
        tracked_proc.lazyadmin_run_id = Some(RunId::new("tracked"));
        let tracked_key = tracked_proc.key.clone();
        snap.processes.push(tracked_proc);
        snap.listeners.push(listener_for_process(tracked_key, 8083));

        let vm = build_view_model(&snap, 120, false, "");
        let mut theme = Theme::default_dark();
        theme.fallback_palette = PaletteMode::Monochrome;
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &theme,
                    RenderContext {
                        view: ViewKind::Listeners,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        for glyph in ["●", "◐", "┃", "▎"] {
            assert!(
                text.contains(glyph),
                "missing {glyph} in monochrome render: {text}"
            );
        }
        assert!(
            text.contains("public"),
            "public count/label missing: {text}"
        );
        assert!(text.contains("LAN"), "LAN count/label missing: {text}");
    }

    #[test]
    fn visual_signal_glyphs_reach_process_and_workload_surfaces() {
        let mut snap = build_empty_snapshot();
        let project_id = lazyadmin_core::model::ProjectId::new("project");
        snap.projects.push(lazyadmin_core::model::Project {
            id: project_id.clone(),
            root: PathBuf::from("/repo"),
            name: "repo".into(),
            markers: Vec::new(),
            git_remote: None,
            package_manager: Some("cargo".into()),
            dev_commands: Vec::new(),
            provenance: Vec::new(),
        });

        let proc = process(4101, None, 1);
        let proc_key = proc.key.clone();
        snap.processes.push(proc);
        snap.workloads.push(lazyadmin_core::model::Workload {
            id: lazyadmin_core::model::WorkloadId::new("workload"),
            display_name: "api".into(),
            runtime: lazyadmin_core::model::RuntimeKind::Direct,
            state: lazyadmin_core::model::WorkloadState::Running,
            pids: vec![proc_key.clone()],
            listeners: Vec::new(),
            project: Some(project_id),
            manager: None,
            source: None,
            actions: Vec::new(),
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: Vec::new(),
        });
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "TEST_WARNING".into(),
            message: "process warning".into(),
            entity: Some(EntityRef::Process(proc_key)),
            provenance: Vec::new(),
        });

        let vm = build_view_model(&snap, 120, false, "");
        let mut theme = Theme::default_dark();
        theme.fallback_palette = PaletteMode::Monochrome;

        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &theme,
                    RenderContext {
                        view: ViewKind::Processes,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let process_text = format!("{:?}", terminal.backend().buffer());
        assert!(
            process_text.contains("▎"),
            "project process marker missing: {process_text}"
        );
        assert!(
            process_text.contains("⚠"),
            "process warning glyph missing: {process_text}"
        );

        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &theme,
                    RenderContext {
                        view: ViewKind::Workloads,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let workload_text = format!("{:?}", terminal.backend().buffer());
        assert!(
            workload_text.contains("▎"),
            "project workload marker missing: {workload_text}"
        );
    }

    #[test]
    fn view_kind_public_programmatic_entry_still_filters_public_rows() {
        let mut snap = build_empty_snapshot();
        let proc = process(3001, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        let mut public = listener_for_process(key, 8080);
        public.exposure = Exposure::Public;
        public.bind_addr = Some("0.0.0.0".into());
        snap.listeners.push(public);
        let private_proc = process(3002, None, 2);
        let private_key = private_proc.key.clone();
        snap.processes.push(private_proc);
        snap.listeners.push(listener_for_process(private_key, 8081));

        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Public,
            listener_filter: ListenerFilter::Public,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        assert_eq!(visible_row_indices(&app).len(), 1);
        assert_eq!(selected_row(&app).unwrap().port, Some(8080));
    }

    #[test]
    fn doctor_view_renders_actionable_warning_rows() {
        let mut snap = build_empty_snapshot();
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Error,
            code: "DEGRADED_PROCFS".into(),
            message: "permission denied".into(),
            entity: None,
            provenance: vec![],
        });
        let vm = build_view_model(&snap, 120, false, "");
        assert_eq!(vm.doctor.rows[0].check, "DEGRADED_PROCFS");
        assert_eq!(vm.doctor.rows[0].details, "DEGRADED_PROCFS");
        assert_eq!(vm.doctor.rows[0].count, 1);
        assert!(
            vm.doctor.rows[0]
                .suggested_action
                .contains("inspect details")
        );
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Doctor,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("1 actionable"));
        assert!(text.contains("Doctor"));
        assert!(!text.contains("PUBLICPUBLIC"));
    }

    /// Doctor entity column must resolve EntityRef references against the
    /// snapshot rather than `Debug`-formatting the raw enum variant. Concretely
    /// a `Listener(...)` reference becomes `listener bind:port`, a
    /// `Process(...)` becomes `pid <n> <command>`, etc. — actionable info, not
    /// `Listener(ListenerId("abc"))`.
    #[test]
    fn doctor_grouping_collapsed_by_default_for_noise() {
        let mut snap = build_empty_snapshot();
        for _ in 0..2 {
            snap.warnings.push(lazyadmin_core::model::Warning {
                severity: WarningSeverity::Warning,
                code: "fd_permission_denied".into(),
                message: "permission denied reading file descriptor".into(),
                entity: None,
                provenance: vec![],
            });
        }
        let vm = build_view_model(&snap, 120, false, "");
        assert_eq!(vm.doctor.rows.len(), 1);
        assert!(vm.doctor.rows[0].is_group);
        assert!(!vm.doctor.rows[0].expanded);
        assert!(
            vm.doctor.rows[0]
                .suggested_action
                .contains("collapsed noise")
        );
    }

    #[test]
    fn doctor_grouping_expand_renders_individual_rows() {
        let mut snap = build_empty_snapshot();
        for _ in 0..2 {
            snap.warnings.push(lazyadmin_core::model::Warning {
                severity: WarningSeverity::Warning,
                code: "fd_permission_denied".into(),
                message: "permission denied reading file descriptor".into(),
                entity: None,
                provenance: vec![],
            });
        }
        let mut toggled = HashSet::new();
        toggled.insert(doctor_group_key(
            "fd_permission_denied",
            &WarningSeverity::Warning,
        ));
        let vm = build_view_model_with_state(
            &snap,
            120,
            false,
            "",
            None,
            &HashSet::new(),
            None,
            &toggled,
            DoctorSeverityFilter::All,
        );
        assert_eq!(vm.doctor.rows.len(), 3);
        assert!(vm.doctor.rows[0].expanded);
        assert!(!vm.doctor.rows[1].is_group);
        assert!(vm.doctor.rows[1].check.starts_with('↳'));
    }

    #[test]
    fn doctor_no_column_truncates_midword_at_160_cols() {
        let mut snap = build_empty_snapshot();
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "PUBLIC".into(),
            message: "permission denied reading file descriptor".into(),
            entity: None,
            provenance: vec![],
        });
        let vm = build_view_model(&snap, 160, false, "");
        let text = render_doctor_text(&vm, 160, 20);
        assert!(!text.contains("permission denied readin"));
        assert!(!text.contains("inspect details and sour"));
    }

    #[test]
    fn doctor_affirmative_empty_state_present() {
        let vm = build_view_model(&build_empty_snapshot(), 120, false, "");
        let text = render_doctor_text(&vm, 120, 20);
        assert!(text.contains("Everything's clean"));
    }

    #[test]
    fn doctor_severity_filter_chip_changes_count() {
        let mut snap = build_empty_snapshot();
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "PUBLIC".into(),
            message: "public listener".into(),
            entity: None,
            provenance: vec![],
        });
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Info,
            code: "possible_dual_stack".into(),
            message: "possible dual stack".into(),
            entity: None,
            provenance: vec![],
        });
        let warning_vm = build_view_model_with_state(
            &snap,
            120,
            false,
            "",
            None,
            &HashSet::new(),
            None,
            &HashSet::new(),
            DoctorSeverityFilter::Warning,
        );
        let info_vm = build_view_model_with_state(
            &snap,
            120,
            false,
            "",
            None,
            &HashSet::new(),
            None,
            &HashSet::new(),
            DoctorSeverityFilter::Info,
        );
        assert_eq!(warning_vm.doctor.rows[0].details, "PUBLIC");
        assert_eq!(info_vm.doctor.rows[0].details, "possible_dual_stack");
        assert_ne!(warning_vm.doctor.rows.len(), info_vm.doctor.rows.len());
    }

    fn render_doctor_text(vm: &ViewModel, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Doctor,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    #[test]
    fn doctor_entity_column_resolves_to_human_labels() {
        let mut snap = build_empty_snapshot();
        let proc = process(4242, None, 1);
        let proc_key = proc.key.clone();
        snap.processes.push(proc);
        let listener = listener_for_process(proc_key.clone(), 8443);
        let listener_id = listener.id.clone();
        snap.listeners.push(listener);
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "PUBLIC".into(),
            message: "exposed to LAN".into(),
            entity: Some(EntityRef::Listener(listener_id)),
            provenance: vec![],
        });
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Info,
            code: "ZOMBIE".into(),
            message: "process is a zombie".into(),
            entity: Some(EntityRef::Process(proc_key)),
            provenance: vec![],
        });
        let vm = build_view_model(&snap, 120, false, "");
        let listener_row = vm
            .doctor
            .rows
            .iter()
            .find(|row| row.details == "PUBLIC")
            .expect("PUBLIC row");
        assert!(
            listener_row.entity.contains("127.0.0.1:8443"),
            "listener entity not resolved: {}",
            listener_row.entity
        );
        assert!(
            listener_row.entity.starts_with("listener "),
            "listener entity missing prefix: {}",
            listener_row.entity
        );
        let process_row = vm
            .doctor
            .rows
            .iter()
            .find(|row| row.details == "ZOMBIE")
            .expect("ZOMBIE row");
        assert!(
            process_row.entity.contains("4242"),
            "process entity not resolved: {}",
            process_row.entity
        );
        // No row should leak the raw Debug shape of the enum.
        for row in &vm.doctor.rows {
            assert!(!row.entity.contains("ListenerId("));
            assert!(!row.entity.contains("ProcessKey {"));
        }
    }

    /// Severity classification must come from a real `match` on
    /// `WarningSeverity`, not a `format!("{:?}")` string compare. This test
    /// exhausts every variant and asserts each lands in the matching counter,
    /// so renaming a variant is a compile error rather than a silent drift to
    /// `info_count`.
    #[test]
    fn doctor_severity_classification_is_exhaustive() {
        let mut snap = build_empty_snapshot();
        for severity in [
            WarningSeverity::Error,
            WarningSeverity::Warning,
            WarningSeverity::Info,
        ] {
            snap.warnings.push(lazyadmin_core::model::Warning {
                severity,
                code: "X".into(),
                message: "x".into(),
                entity: None,
                provenance: vec![],
            });
        }
        let vm = build_view_model(&snap, 120, false, "");
        assert_eq!(vm.doctor.error_count, 1);
        assert_eq!(vm.doctor.warning_count, 1);
        assert_eq!(vm.doctor.info_count, 1);
        let kinds: Vec<DoctorSeverity> = vm.doctor.rows.iter().map(|r| r.severity_kind).collect();
        assert!(kinds.contains(&DoctorSeverity::Error));
        assert!(kinds.contains(&DoctorSeverity::Warning));
        assert!(kinds.contains(&DoctorSeverity::Info));
        // The legacy stringly-typed field stays in lock-step with the enum so
        // the JSON contract doesn't drift.
        for row in &vm.doctor.rows {
            assert_eq!(row.severity, row.severity_kind.label());
        }
    }

    #[test]
    fn help_lines_skip_empty_bindings() {
        let mut keybindings = ResolvedKeybindings {
            bindings: BTreeMap::new(),
        };
        keybindings.bindings.insert("toggle_filter".into(), vec![]);
        keybindings.bindings.insert("quit".into(), vec!["q".into()]);
        let lines = help_lines(&keybindings);
        assert!(lines.contains(&"quit: q".into()));
        assert!(!lines.iter().any(|line| line.contains("toggle_filter")));
        assert!(lines.iter().any(|line| line.contains("listeners chips")));
    }

    #[test]
    fn live_refresh_event_debounce_and_cap() {
        let now = Instant::now();
        let mut state = LiveRefreshState::new(Duration::from_millis(100), 1);
        state.on_event(&DiscoveryEvent::heartbeat("procfs"), now);
        assert!(!state.should_refresh(now + Duration::from_millis(50)));
        assert!(state.should_refresh(now + Duration::from_millis(120)));
        state.on_event(
            &DiscoveryEvent::heartbeat("procfs"),
            now + Duration::from_millis(130),
        );
        assert!(!state.should_refresh(now + Duration::from_millis(250)));
        assert_eq!(state.coalesced, 1);
        state.set_dropped(2);
        assert_eq!(state.events_dropped, 2);
    }

    #[test]
    fn live_refresh_degraded_event_sets_footer_pill() {
        let mut state = LiveRefreshState::new(Duration::ZERO, 30);
        state.on_event(
            &DiscoveryEvent::degraded("procfs", "permission denied"),
            Instant::now(),
        );
        assert!(state.degraded.unwrap().contains("permission denied"));
    }

    fn process(pid: i32, ppid: Option<i32>, start: u64) -> lazyadmin_core::model::Process {
        lazyadmin_core::model::Process {
            key: ProcessKey {
                pid,
                boot_id: "boot".into(),
                start_time_ticks: start,
            },
            pid,
            start_time_ticks: start,
            boot_id: "boot".into(),
            user: None,
            exe: None,
            cmdline: vec![format!("p{pid}")],
            cwd: None,
            ppid,
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: None,
            lazyadmin_run_id: None,
            environment: Default::default(),
            provenance: vec![],
        }
    }

    fn listener_for_process(key: ProcessKey, port: u16) -> Listener {
        let now = chrono::Utc::now();
        Listener {
            id: ListenerId::new(format!("tcp:127.0.0.1:{port}")),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("127.0.0.1".into()),
            port: Some(port),
            path: None,
            state: ListenerState::Listen,
            netns: "default".into(),
            socket_inode: None,
            exposure: Exposure::Loopback,
            owners: vec![EntityRef::Process(key)],
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: now,
            last_seen: now,
            dual_stack_state: DualStackState::NotApplicable,
        }
    }

    fn render_inspector_text(vm: &ViewModel, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_inspector(vm, f, f.area(), &Theme::default_dark(), true))
            .unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    #[test]
    fn inspector_lists_related_listeners_at_38_col_width() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key.clone(), 8443));
        snap.listeners.push(listener_for_process(key, 8444));
        let vm = build_view_model(&snap, 38, false, "");
        assert!(vm.inspector.sections.iter().any(|s| s.heading == "RELATED"));
        let text = render_inspector_text(&vm, 38, 18);
        assert!(text.contains("RELATED"), "{text}");
        assert!(text.contains("[1]"), "{text}");
        assert!(text.contains("8444"), "{text}");
    }

    #[test]
    fn inspector_jump_shortcut_selects_related_listener() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key.clone(), 8443));
        snap.listeners.push(listener_for_process(key, 8444));
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Listeners,
            pane: Pane::Inspector,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        assert_eq!(app.vm.inspector.jump_targets.len(), 1);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
            120,
        );
        assert_eq!(selected_row(&app).unwrap().port, Some(8444));
    }

    #[test]
    fn inspector_view_all_related_filters_listener_table() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        for offset in 0..11 {
            snap.listeners
                .push(listener_for_process(key.clone(), 8400 + offset));
        }
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Listeners,
            pane: Pane::Inspector,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        let text = render_inspector_text(&app.vm, 120, 40);
        assert!(text.contains("[v] view all related"), "{text}");

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Listeners);
        assert_eq!(visible_row_indices(&app).len(), 10);
        assert!(app.related_listener_filter.is_some());
        assert_eq!(selected_row(&app).unwrap().port, Some(8401));
    }

    #[test]
    fn inspector_action_key_opens_confirmation_with_command_preview() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key, 8443));
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Listeners,
            pane: Pane::Inspector,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            120,
        );
        let confirmation = app.confirmation.as_ref().unwrap();
        assert_eq!(confirmation.required, "free");
        assert!(confirmation.command_preview.contains("lazyadmin free 8443"));

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("Confirm action"), "{text}");
        assert!(text.contains("lazyadmin free 8443"), "{text}");
        assert!(text.contains("Type 'free'"), "{text}");
    }

    #[test]
    fn inspector_disabled_action_key_shows_reason_without_confirmation() {
        let mut snap = build_empty_snapshot();
        let proc = process(5555, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        let mut app = App {
            vm: build_view_model_with_state(
                &snap,
                120,
                false,
                "",
                Some(key),
                &HashSet::new(),
                None,
                &HashSet::new(),
                DoctorSeverityFilter::All,
            ),
            snapshot: snap,
            active_view: ViewKind::ProcessTree,
            pane: Pane::Inspector,
            selected_process: None,
            ..Default::default()
        };
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT),
            120,
        );
        assert!(app.confirmation.is_none());
        assert!(
            app.status
                .as_deref()
                .unwrap_or_default()
                .contains("logs only available"),
            "{:?}",
            app.status
        );
    }

    #[test]
    fn inspector_listener_id_full_string_present_at_38_col_width() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key, 8443));
        let vm = build_view_model(&snap, 38, false, "");
        let plain = vm.inspector.lines.join("\n");
        assert!(plain.contains("tcp:127.0.0.1:8443"), "{plain}");
        let text = render_inspector_text(&vm, 38, 18);
        assert!(text.contains("tcp:127.0.0.1:8443"), "{text}");
        assert!(!text.contains("tcp:127.0.0.1…"), "{text}");
    }

    #[test]
    fn inspector_full_cmdline_wraps_without_truncation() {
        let mut snap = build_empty_snapshot();
        let mut proc = process(5555, None, 1);
        proc.cmdline = vec![
            "node".into(),
            "packages/web/dev-server-with-a-very-specific-script-name.js".into(),
            "--flag=value".into(),
        ];
        let key = proc.key.clone();
        snap.processes.push(proc);
        let vm = build_view_model_with_state(
            &snap,
            60,
            false,
            "",
            Some(key),
            &HashSet::new(),
            None,
            &HashSet::new(),
            DoctorSeverityFilter::All,
        );
        let plain = vm.inspector.lines.join("\n");
        assert!(
            plain.contains("packages/web/dev-server-with-a-very-specific-script-name.js"),
            "{plain}"
        );
        assert!(!plain.contains('…'), "{plain}");
    }

    #[test]
    fn inspector_disabled_action_renders_with_reason() {
        let mut snap = build_empty_snapshot();
        let proc = process(5555, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        let vm = build_view_model_with_state(
            &snap,
            120,
            false,
            "",
            Some(key),
            &HashSet::new(),
            None,
            &HashSet::new(),
            DoctorSeverityFilter::All,
        );
        let text = render_inspector_text(&vm, 120, 18);
        assert!(text.contains("[L] logs"), "{text}");
        assert!(text.contains("disabled"), "{text}");
        assert!(text.contains("tracked runs"), "{text}");
    }

    #[test]
    fn inspector_no_dash_rows_visible() {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key, 8443));
        let vm = build_view_model(&snap, 120, false, "");
        let rendered = vm.inspector.lines.join("\n");
        assert!(!rendered.contains("Warnings: -"), "{rendered}");
        assert!(!rendered.contains("Project: -"), "{rendered}");
        assert!(!rendered.contains("unavailable"), "{rendered}");
    }

    fn app_with_listener(port: u16) -> App {
        let mut snap = build_empty_snapshot();
        let proc = process(4321, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key, port));
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Everything,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        app
    }

    #[test]
    fn kill_confirmation_blocks_global_keys_and_renders_target() {
        let mut app = app_with_listener(8080);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        assert!(app.confirmation.as_ref().unwrap().target.contains("8080"));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            120,
        );
        assert!(!app.should_quit, "q must be suppressed while confirming");
        assert_eq!(app.confirmation.as_ref().unwrap().typed, "q");

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_app(f, &app)).unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(text.contains("Confirm action"));
        assert!(text.contains("8080"));
        assert!(text.contains("Type 'kill'"));
        let footer_line = text.lines().last().unwrap_or_default();
        assert!(
            !footer_line.contains("confirm kill"),
            "confirmation hint belongs in modal, not footer: {footer_line}"
        );
    }

    #[test]
    fn kill_confirmation_esc_cancels_and_correct_text_dry_runs() {
        let mut app = app_with_listener(8080);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            120,
        );
        assert!(app.confirmation.is_none());
        assert!(app.status.as_deref().unwrap().contains("cancelled"));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        for c in "kill".chars() {
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                120,
            );
        }
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            120,
        );
        assert!(app.confirmation.is_none());
        let status = app.status.as_deref().unwrap();
        assert!(status.contains("Dry run: Kill"));
        assert!(status.contains("8080"));
    }

    /// Ctrl+C is the universal cancel reflex; in confirmation mode it must
    /// behave like Esc, not like a literal `c` keystroke that grows the typed
    /// buffer.
    #[test]
    fn ctrl_c_cancels_confirmation_instead_of_appending_c() {
        let mut app = app_with_listener(8080);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        assert!(app.confirmation.is_some());
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            120,
        );
        assert!(
            app.confirmation.is_none(),
            "Ctrl+C must close the confirmation modal"
        );
        let status = app.status.as_deref().unwrap();
        assert!(status.contains("cancelled"), "status: {status}");
    }

    #[test]
    fn advertised_action_keys_show_selected_row_feedback() {
        for (key, expected) in [
            ('r', "Dry run: Restart"),
            ('s', "Dry run: Stop"),
            ('f', "Dry run: Free"),
            ('R', "Dry run: Run"),
            ('e', "edit not implemented"),
        ] {
            let mut app = app_with_listener(8080);
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                120,
            );
            let status = app.status.as_deref().unwrap_or_default();
            assert!(status.contains(expected), "{key}: {status}");
            assert!(status.contains("8080"), "{key}: {status}");
        }
    }

    #[test]
    fn row_owner_labels_are_compact_and_readable() {
        let mut snap = build_empty_snapshot();
        let proc = process(1234, None, 1);
        let key = proc.key.clone();
        snap.processes.push(proc);
        snap.listeners.push(listener_for_process(key, 8080));
        let vm = build_view_model(&snap, 120, false, "");
        assert_eq!(vm.rows[0].owner, "p1234 pid 1234");
        assert!(!vm.rows[0].owner.contains("ProcessKey"));
        assert_eq!(vm.rows[0].runtime, "direct");
        assert_eq!(vm.rows[0].exposure, "loopback");
    }

    #[test]
    fn row_scrolling_updates_selection_and_inspector() {
        let mut snap = build_empty_snapshot();
        for offset in 0..3 {
            let proc = process(2000 + offset, None, offset as u64 + 1);
            let key = proc.key.clone();
            snap.processes.push(proc);
            snap.listeners
                .push(listener_for_process(key, 8000 + offset as u16));
        }
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            active_view: ViewKind::Everything,
            ..Default::default()
        };
        sync_row_selection(&mut app);
        assert!(app.vm.inspector.title.contains("8000"));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.selected_row, 1);
        assert!(app.vm.inspector.title.contains("8001"));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.selected_row, 2);
        assert!(app.vm.inspector.title.contains("8002"));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.selected_row, 2);
    }

    #[test]
    fn tab_and_shift_tab_change_focused_pane() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            ..Default::default()
        };
        assert_eq!(app.pane, Pane::Rows);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.pane, Pane::Inspector);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            120,
        );
        assert_eq!(app.pane, Pane::Rows);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            120,
        );
        assert_eq!(app.pane, Pane::Groups);
    }

    #[test]
    fn view_rail_is_collapsed_to_runtime_entries() {
        let group_list = groups(false);
        assert_eq!(group_list.len(), 6);
        assert_eq!(
            group_list,
            vec![
                "Overview",
                "Listeners",
                "Workloads",
                "Processes",
                "Doctor",
                "Metrics"
            ]
        );
        for removed in [
            "Ports",
            "Public listeners",
            "Conflicts",
            "Orphans",
            "Tracked runs",
            "Docker/Compose",
            "Podman",
            "systemd:user",
            "systemd:system [hidden]",
            "Direct processes",
        ] {
            assert!(!group_list.iter().any(|item| item == removed));
            assert!(group_view_kind(removed).is_none());
        }

        let snap = build_empty_snapshot();
        let vm = build_view_model(&snap, 120, false, "");
        let backend = TestBackend::new(120, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Overview,
                        active_pane: Pane::Groups,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let combined = format!("{:?}", terminal.backend().buffer());
        assert!(
            combined.contains("› Overview"),
            "active marker missing: {combined}"
        );
        assert!(
            combined.contains("  Listeners"),
            "listeners rail entry missing: {combined}"
        );
        assert!(!combined.contains("Docker/Compose"));
    }

    #[test]
    fn rail_has_at_most_eight_entries() {
        assert!(groups(false).len() <= 8);
    }

    #[test]
    fn rail_render_has_no_hidden_or_removed_entries_at_common_widths() {
        let snap = build_empty_snapshot();
        for width in [70, 90, 120, 160] {
            let vm = build_view_model(&snap, width, false, "");
            let backend = TestBackend::new(width, 28);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| {
                    render_view_kind(
                        &vm,
                        f,
                        f.area(),
                        &Theme::default_dark(),
                        RenderContext {
                            view: ViewKind::Overview,
                            active_pane: Pane::Groups,
                            keybindings: None,
                            selected_row: 0,
                            overview_hint_visible: false,
                            listener_filter: ListenerFilter::All,
                            listeners_hint_visible: false,
                            related_listener_filter: None,
                        },
                    )
                })
                .unwrap();
            let text = format!("{:?}", terminal.backend().buffer());
            for removed in [
                "[hidden]",
                "Docker/Compose",
                "Podman",
                "systemd:user",
                "systemd:system",
                "Direct processes",
                "Public listeners",
                "Tracked runs",
            ] {
                assert!(
                    !text.contains(removed),
                    "width {width} still rendered removed rail entry {removed}: {text}"
                );
            }
        }
    }

    #[test]
    fn down_arrow_uses_collapsed_rail_order() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            pane: Pane::Groups,
            active_view: ViewKind::Workloads,
            ..Default::default()
        };
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Processes);
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Workloads);
    }

    #[test]
    fn focused_views_pane_arrow_keys_change_active_view() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            pane: Pane::Groups,
            ..Default::default()
        };

        assert_eq!(app.active_view, ViewKind::Overview);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Listeners);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Overview);
    }

    #[test]
    fn tab_cycles_views_when_view_pane_is_hidden() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 70, false, ""),
            snapshot: snap,
            ..Default::default()
        };
        assert_eq!(app.vm.layout, LayoutMode::SinglePane);

        assert_eq!(app.active_view, ViewKind::Overview);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            70,
        );
        assert_eq!(app.active_view, ViewKind::Listeners);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            70,
        );
        assert_eq!(app.active_view, ViewKind::Overview);
    }

    #[test]
    fn process_tree_shape_selection_and_pid_reuse() {
        let mut snap = build_empty_snapshot();
        snap.processes = vec![
            process(1, None, 1),
            process(2, Some(1), 2),
            process(2, Some(1), 3),
        ];
        let selected = snap.processes[1].key.clone();
        let tree = build_process_tree(&snap, Some(selected.clone()));
        assert_eq!(tree.rows.len(), 3);
        assert_eq!(tree.selected, Some(selected));
        assert_ne!(tree.rows[1].key, tree.rows[2].key);
    }

    #[test]
    fn process_tree_expand_collapse_preserves_selection_and_inspector() {
        let mut snap = build_empty_snapshot();
        snap.processes = vec![process(1, None, 1), process(2, Some(1), 2)];
        let selected = snap.processes[0].key.clone();
        let mut collapsed = HashSet::new();
        collapsed.insert(selected.clone());
        let vm = build_view_model_with_state(
            &snap,
            120,
            false,
            "",
            Some(selected.clone()),
            &collapsed,
            None,
            &HashSet::new(),
            DoctorSeverityFilter::All,
        );
        assert_eq!(vm.process_tree.selected, Some(selected));
        assert_eq!(vm.process_tree.rows.len(), 1);
        assert!(vm.inspector.title.contains("pid 1"));
    }

    #[test]
    fn metrics_counts_and_rates() {
        let prev = build_empty_snapshot();
        let mut snap = build_empty_snapshot();
        snap.warnings.push(lazyadmin_core::model::Warning {
            severity: WarningSeverity::Warning,
            code: "W".into(),
            message: "warn".into(),
            entity: None,
            provenance: vec![],
        });
        let metrics = build_metrics(&snap, Some(&prev));
        assert_eq!(metrics.tracked_runs, 0);
        assert!(metrics.event_rate.iter().all(|v| *v < 10));
        assert_eq!(metrics.warnings_by_severity[0].0, "Warning");
    }

    #[test]
    fn metrics_adapter_ring_populates_health_rows() {
        let mut ring = AdapterEventRing::with_capacity(8);
        let now = Instant::now();
        ring.record(&DiscoveryEvent::heartbeat("procfs"), now);
        ring.record(
            &DiscoveryEvent::heartbeat("procfs"),
            now + Duration::from_millis(50),
        );
        ring.set_dropped("procfs", 3);
        let metrics = build_metrics_with_adapters(
            &build_empty_snapshot(),
            None,
            ring.metrics(now + Duration::from_millis(100)),
        );
        assert_eq!(metrics.adapters[0].adapter, "procfs");
        assert_eq!(metrics.adapters[0].throughput, 2);
        assert_eq!(metrics.adapters[0].drops, 3);
        assert!(!metrics.adapters[0].sparkline.is_empty());
    }

    fn render_metrics_text(metrics: MetricsVm, width: u16, height: u16) -> String {
        let vm = ViewModel {
            width,
            layout: LayoutMode::ThreePane,
            metrics,
            ..Default::default()
        };
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_metrics(&vm, f, f.area(), &Theme::default_dark(), true);
            })
            .unwrap();
        format!("{:?}", terminal.backend().buffer())
    }

    #[test]
    fn metrics_events_dropped_rate_with_nontrivial_denominator_renders_correctly() {
        let metrics = MetricsVm {
            events_dropped: 27,
            adapters: vec![AdapterMetricVm {
                adapter: "procfs".into(),
                latency_ms: None,
                throughput: 4173,
                drops: 27,
                sparkline: vec![1, 2, 3],
            }],
            event_rate: vec![4173],
            ..Default::default()
        };
        let text = render_metrics_text(metrics, 140, 30);
        assert!(text.contains("27 / 4200"), "{text}");
        assert!(text.contains("0.6%"), "{text}");
        assert!(text.contains("last 60s"), "{text}");
    }

    #[test]
    fn metrics_empty_adapter_event_rate_shows_idle_message() {
        let text = render_metrics_text(MetricsVm::default(), 140, 24);
        assert!(
            text.contains("No events in last 60s"),
            "idle copy missing: {text}"
        );
        assert!(text.contains("normal"), "normality copy missing: {text}");
    }

    #[test]
    fn metrics_listener_histogram_axis_uses_full_words() {
        let metrics = MetricsVm {
            listeners_loopback: 3,
            listeners_public: 2,
            listeners_conflicts: 1,
            listeners_orphans: 1,
            ..Default::default()
        };
        let text = render_metrics_text(metrics, 140, 30);
        for label in ["Listeners", "Public", "Conflicts", "Orphans"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
    }

    #[test]
    fn theme_builtins_validate_and_downgrade() {
        for name in [
            "default-dark",
            "night-owl",
            "default-light",
            "night-owl-light",
            "high-contrast",
            "colorblind-safe",
            "solarized-dark",
        ] {
            let mut theme = Theme::builtin(name).unwrap();
            theme.validate().unwrap();
            assert_eq!(theme.name, name);
            for slot in [
                &theme.risk_public,
                &theme.risk_lan,
                &theme.risk_loopback,
                &theme.marker_conflict,
                &theme.marker_tracked,
                &theme.marker_project,
                &theme.system_noise,
                &theme.pip_ok,
                &theme.pip_warn,
                &theme.pip_error,
            ] {
                ColorSpec::parse(&slot.0).unwrap();
            }
        }
        assert!(ColorSpec::parse("not-a-color").is_err());
        let (theme, hint) = Theme::builtin("solarized-dark")
            .unwrap()
            .downgrade_for_colors(16);
        assert_eq!(theme.fallback_palette, PaletteMode::Sixteen);
        assert!(hint.unwrap().contains("limited color"));
    }

    #[test]
    fn themes_validation_alias() {
        theme_builtins_validate_and_downgrade();
    }

    #[test]
    fn status_channel_routes_to_toast_queue() {
        let mut app = App::default();
        app.push_status(
            StatusChannel::Toast {
                ttl: Duration::from_secs(2),
            },
            "diagnostic copied",
        );
        assert_eq!(app.status.as_deref(), Some("diagnostic copied"));
        assert_eq!(app.toasts.len(), 1);
        assert_eq!(app.toasts[0].ttl, Duration::from_secs(2));
        assert!(app.toasts[0].created_at.is_some());
    }

    #[test]
    fn toast_dismisses_after_ttl() {
        let mut app = App::default();
        app.toasts.push_back(Toast {
            message: "short lived".into(),
            ttl: Duration::from_secs(2),
            created_at: Some(Instant::now() - Duration::from_secs(3)),
        });
        assert!(active_toast_message(&app, Instant::now()).is_none());
    }

    #[test]
    fn toast_dismissal_paused_during_input() {
        let mut app = App {
            mode: InputMode::Filter,
            ..Default::default()
        };
        app.toasts.push_back(Toast {
            message: "filtering".into(),
            ttl: Duration::from_secs(2),
            created_at: Some(Instant::now() - Duration::from_secs(3)),
        });
        assert_eq!(
            active_toast_message(&app, Instant::now()).as_deref(),
            Some("filtering")
        );
    }

    #[test]
    fn no_residue_when_long_message_replaced_by_short_message() {
        let mut long = App {
            vm: build_view_model(&build_empty_snapshot(), 120, false, ""),
            active_view: ViewKind::Listeners,
            status: Some("this is a very long transient status message".into()),
            ..Default::default()
        };
        long.snapshot = build_empty_snapshot();
        let short = App {
            vm: build_view_model(&build_empty_snapshot(), 120, false, ""),
            active_view: ViewKind::Listeners,
            ..Default::default()
        };
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_app(f, &long)).unwrap();
        terminal.draw(|f| render_app(f, &short)).unwrap();
        let text = format!("{:?}", terminal.backend().buffer());
        assert!(!text.contains("very long transient"), "{text}");
        assert!(text.contains("[?] help"), "{text}");
    }

    #[test]
    fn footer_padded_to_full_width_in_every_layout() {
        let mut line = pad_to_width(Line::from(Span::raw("[q] quit")), 20);
        assert_eq!(line.width(), 20);
        line = pad_to_width(line, 8);
        assert_eq!(line.width(), 20);
    }

    #[test]
    fn header_pip_renders_drop_count_only_when_nonzero() {
        let mut clean = build_empty_snapshot();
        clean.generated_at = chrono::Utc::now();
        let clean_vm = build_view_model(&clean, 120, false, "");
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &clean_vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Overview,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let clean_text = format!("{:?}", terminal.backend().buffer());
        assert!(!clean_text.contains("events dropped"), "{clean_text}");
        assert!(clean_text.contains("healthy"), "{clean_text}");

        let mut dropped = clean;
        dropped.metadata = Some(lazyadmin_core::model::SnapshotMetadata {
            events_dropped: Some(7),
        });
        let dropped_vm = build_view_model(&dropped, 120, false, "");
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_view_kind(
                    &dropped_vm,
                    f,
                    f.area(),
                    &Theme::default_dark(),
                    RenderContext {
                        view: ViewKind::Overview,
                        active_pane: Pane::Rows,
                        keybindings: None,
                        selected_row: 0,
                        overview_hint_visible: false,
                        listener_filter: ListenerFilter::All,
                        listeners_hint_visible: false,
                        related_listener_filter: None,
                    },
                )
            })
            .unwrap();
        let dropped_text = format!("{:?}", terminal.backend().buffer());
        assert!(dropped_text.contains("events dropped 7"), "{dropped_text}");
    }

    #[test]
    fn theme_file_missing_keys_inherits_default_dark() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("minimal.toml");
        std::fs::write(
            &path,
            r##"
name = "minimal"
accent = "#123456"
"##,
        )
        .unwrap();
        let theme = Theme::load(None, Some(&path)).unwrap();
        assert_eq!(theme.name, "minimal");
        assert_eq!(theme.accent, ColorSpec("#123456".into()));
        assert_eq!(theme.base_bg, Theme::default_dark().base_bg);
    }

    #[test]
    fn keybindings_help_overlay_reflects_overrides() {
        let mut cfg = lazyadmin_core::config::Config::default();
        cfg.ui
            .keybindings
            .overrides
            .insert("quit".into(), "Q".into());
        let keybindings = ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(
            key_to_command_with_bindings(
                KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE),
                &keybindings,
            ),
            Some(Command::Quit)
        );
        assert_eq!(
            key_to_command_with_bindings(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &keybindings,
            ),
            None
        );
        assert!(
            help_lines(&keybindings)
                .iter()
                .any(|l| l.contains("quit: Q"))
        );
    }

    #[test]
    fn keybindings_override_dispatch_for_help_search_toggle() {
        let mut cfg = lazyadmin_core::config::Config::default();
        cfg.ui
            .keybindings
            .overrides
            .insert("help".into(), "h".into());
        cfg.ui
            .keybindings
            .overrides
            .insert("filter".into(), "F".into());
        cfg.ui
            .keybindings
            .overrides
            .insert("toggle_system".into(), "T".into());
        let keybindings = ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(
            key_to_command_with_bindings(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                &keybindings
            ),
            Some(Command::Help)
        );
        assert_eq!(
            key_to_command_with_bindings(
                KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE),
                &keybindings
            ),
            Some(Command::Filter)
        );
        assert_eq!(
            key_to_command_with_bindings(
                KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE),
                &keybindings
            ),
            Some(Command::ToggleSystem)
        );
    }

    #[test]
    fn help_palette_open_copy_behaviors() {
        assert!(palette_entries("reload").contains(&"reload"));
        let dir = tempfile::tempdir().unwrap();
        let path = copy_diagnostic_fallback("# diag", Some(dir.path())).unwrap();
        assert!(path.exists());
        let row = RowVm {
            id: "x".into(),
            port: Some(80),
            bind: "0.0.0.0".into(),
            owner: "o".into(),
            runtime: "direct".into(),
            exposure: "loopback".into(),
            project: "-".into(),
            badges: vec![],
            is_conflict: false,
            is_orphan: false,
            is_tracked: false,
            is_project: false,
            is_system: false,
            search_text: "".into(),
        };
        assert!(open_url_for_row(&row, false).is_err());
        let mut row = row;
        row.bind = "127.0.0.1".into();
        row.port = Some(9999);
        assert!(open_url_for_row(&row, false).is_err());
        row.bind = "127.0.2.3".into();
        row.port = Some(8080);
        assert!(open_url_for_row(&row, false).is_ok());
    }

    #[test]
    fn palette_reload_applies_config_callback() {
        let mut app = App {
            config_reload: Some(Box::new(|| {
                let mut cfg = lazyadmin_core::config::Config::default();
                cfg.ui
                    .keybindings
                    .overrides
                    .insert("quit".into(), "Q".into());
                Ok((
                    Theme::builtin("high-contrast").unwrap(),
                    ResolvedKeybindings::from_config(&cfg).unwrap(),
                ))
            })),
            ..Default::default()
        };
        run_palette_command(&mut app, "reload", 120);
        assert_eq!(app.theme.name, "high-contrast");
        assert_eq!(app.keybindings.bindings["quit"], vec!["Q"]);
        assert_eq!(app.status.as_deref(), Some("config reloaded"));
    }
}
