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
    model::{DiscoveryEvent, EntityRef, Exposure, ProcessKey, Snapshot},
    output::listener_rows,
    snapshot::build_empty_snapshot,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, BorderType, Borders, Cell, Clear, Gauge, List, ListItem,
        Paragraph, Row, Sparkline, Table, TableState, Wrap,
    },
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tracing::{debug, info, info_span};

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
    pub allow_open_non_loopback: bool,
    selected_row: usize,
    selected_process: Option<ProcessKey>,
    collapsed_processes: HashSet<ProcessKey>,
    event_ring: AdapterEventRing,
    config_reload: Option<ConfigReload>,
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
            active_view: ViewKind::Everything,
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
            allow_open_non_loopback: false,
            selected_row: 0,
            selected_process: None,
            collapsed_processes: HashSet::new(),
            event_ring: AdapterEventRing::default(),
            config_reload: None,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewKind {
    #[default]
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
    fallback_palette: Option<PaletteMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteMode {
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
        let mut t = Self {
            name: name.into(),
            base_fg: ColorSpec("#d7e2ed".into()),
            base_bg: ColorSpec("#07131c".into()),
            accent: ColorSpec("#f2b84b".into()),
            ok: ColorSpec("#2dd4bf".into()),
            info: ColorSpec("#7aa2f7".into()),
            warning: ColorSpec("#f6c177".into()),
            degraded: ColorSpec("#c678dd".into()),
            error: ColorSpec("#ff5d5d".into()),
            selection: ColorSpec("#1f4f66".into()),
            footer: ColorSpec("#8aa0b2".into()),
            fallback_palette: PaletteMode::Truecolor,
        };
        match name {
            "default-dark" => Some(t),
            "default-light" => {
                t.base_fg = ColorSpec("#17202a".into());
                t.base_bg = ColorSpec("#f7f3ea".into());
                t.accent = ColorSpec("#875f00".into());
                t.info = ColorSpec("#245c9e".into());
                t.selection = ColorSpec("#d9e8f2".into());
                t.footer = ColorSpec("#53616f".into());
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
    pub fn plan(command: &Command, _row: Option<&RowVm>) -> String {
        format!("Dry run: {command:?} would use lazyadmin-core action planning/execution services")
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewModel {
    pub width: u16,
    pub layout: LayoutMode,
    pub groups: Vec<String>,
    pub rows: Vec<RowVm>,
    pub process_tree: ProcessTreeVm,
    pub metrics: MetricsVm,
    pub inspector: InspectorVm,
    pub hidden_system_count: usize,
    pub degraded: Option<String>,
    pub events_dropped: u64,
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
    pub search_text: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorVm {
    pub title: String,
    pub lines: Vec<String>,
    pub provenance: Vec<String>,
    pub provenance_expanded: bool,
    pub diagnostic_markdown: String,
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
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsVm {
    pub listeners_loopback: usize,
    pub listeners_public: usize,
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
                    .filter(|seen| now.duration_since(**seen) <= Duration::from_secs(1))
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
    )
}

pub fn build_view_model_with_state(
    snapshot: &Snapshot,
    width: u16,
    show_system: bool,
    filter: &str,
    selected_process: Option<ProcessKey>,
    collapsed_processes: &HashSet<ProcessKey>,
    adapter_metrics: Option<Vec<AdapterMetricVm>>,
) -> ViewModel {
    let layout = match width {
        100..=u16::MAX => LayoutMode::ThreePane,
        80..=99 => LayoutMode::InspectorTab,
        60..=79 => LayoutMode::SinglePane,
        _ => LayoutMode::Refuse,
    };
    let mut rows = Vec::new();
    let mut hidden = 0usize;
    let projected_rows = listener_rows(snapshot);
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
        .or_else(|| rows.first().map(inspector_for_row))
        .unwrap_or_else(|| InspectorVm {
            title: "No selection".into(),
            lines: vec!["No workloads/listeners discovered yet".into()],
            provenance: vec![],
            provenance_expanded: false,
            diagnostic_markdown: "# lazyadmin diagnostic\nNo selection\n".into(),
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
    ViewModel {
        width,
        layout,
        groups: groups(show_system),
        rows,
        process_tree,
        metrics: build_metrics_with_adapters(snapshot, None, adapter_metrics.unwrap_or_default()),
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
    let workload = snapshot
        .workloads
        .iter()
        .find(|w| w.pids.contains(&process.key))
        .map(|w| w.display_name.clone())
        .or_else(|| {
            process
                .lazyadmin_run_id
                .as_ref()
                .map(|r| format!("run:{r}"))
        });
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
        warnings: Vec::new(),
        expanded: is_expanded,
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
    MetricsVm {
        listeners_loopback,
        listeners_public,
        workloads_by_runtime: runtimes.into_iter().collect(),
        warnings_by_severity: severities.into_iter().collect(),
        tracked_runs: snapshot.tracked_runs.len(),
        events_dropped: snapshot
            .metadata
            .as_ref()
            .and_then(|m| m.events_dropped)
            .unwrap_or(0),
        event_rate: vec![rate],
        adapters,
    }
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
fn inspector_for_row(row: &RowVm) -> InspectorVm {
    InspectorVm {
        title: format!(
            "Listener {}",
            row.port
                .map(|port| port.to_string())
                .unwrap_or_else(|| short_id(&row.id))
        ),
        lines: vec![
            format!(
                "Port: {}",
                row.port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "-".into())
            ),
            format!("Bind: {}", row.bind),
            format!("Owner: {}", row.owner),
            format!("Runtime: {}", row.runtime),
            format!("Scope: {}", row.exposure),
            format!("Project: {}", row.project),
            "Logs: unavailable for direct processes".into(),
            format!(
                "Warnings: {}",
                if row.badges.is_empty() {
                    "-".into()
                } else {
                    row.badges.join(", ")
                }
            ),
            "Actions: open  logs  restart  stop  free-port".into(),
        ],
        provenance: vec![
            format!("listener id {}", short_id(&row.id)),
            "core snapshot services; confidence best-effort".into(),
        ],
        provenance_expanded: false,
        diagnostic_markdown: format!(
            "# lazyadmin diagnostic\n\n- owner: {}\n- port: {:?}\n- bind: {}\n- runtime: {}\n- scope: {}\n- provenance: core snapshot\n",
            row.owner, row.port, row.bind, row.runtime, row.exposure
        ),
    }
}

fn inspector_for_process(snapshot: &Snapshot, key: &ProcessKey) -> Option<InspectorVm> {
    let process = snapshot.processes.iter().find(|p| &p.key == key)?;
    let ports = snapshot
        .listeners
        .iter()
        .filter(|listener| {
            listener
                .owners
                .iter()
                .any(|owner| matches!(owner, EntityRef::Process(process_key) if process_key == key))
        })
        .filter_map(|listener| listener.port.map(|p| p.to_string()))
        .collect::<Vec<_>>();
    let workload = snapshot
        .workloads
        .iter()
        .find(|workload| workload.pids.contains(key));
    let project = workload
        .and_then(|workload| workload.project.as_ref())
        .and_then(|project_id| {
            snapshot
                .projects
                .iter()
                .find(|project| &project.id == project_id)
        })
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "-".into());
    let tracked = process
        .lazyadmin_run_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| {
            workload
                .and_then(|workload| workload.lazyadmin_run_id.as_ref())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "-".into());
    let runtime = process
        .systemd_unit
        .as_ref()
        .map(|unit| format!("systemd:{unit}"))
        .or_else(|| {
            process
                .container_id
                .as_ref()
                .map(|container| format!("container:{container}"))
        })
        .unwrap_or_else(|| "direct".into());
    Some(InspectorVm {
        title: format!("pid {}", process.pid),
        lines: vec![
            format!(
                "Identity: pid {} start {}",
                process.pid, process.start_time_ticks
            ),
            "State: running".into(),
            format!("Runtime: {runtime}"),
            format!(
                "Ports: {}",
                if ports.is_empty() {
                    "-".into()
                } else {
                    ports.join(", ")
                }
            ),
            format!("Project: {project}"),
            format!("Tracked: {tracked}"),
            format!(
                "Logs: {}",
                process
                    .lazyadmin_run_id
                    .as_ref()
                    .map(|_| "tracked run logs available")
                    .unwrap_or("no direct process log source")
            ),
            "Warnings: -".into(),
            "Actions: open  logs  restart  stop  free-port".into(),
        ],
        provenance: process
            .provenance
            .iter()
            .map(|p| format!("{}: {} ({:?})", p.adapter, p.claim, p.confidence))
            .collect(),
        provenance_expanded: true,
        diagnostic_markdown: format!(
            "# lazyadmin process diagnostic\n\n- pid: {}\n- start_time_ticks: {}\n- runtime: {}\n- ports: {}\n- project: {}\n",
            process.pid,
            process.start_time_ticks,
            runtime,
            if ports.is_empty() {
                "-".into()
            } else {
                ports.join(", ")
            },
            project
        ),
    })
}
fn groups(show_system: bool) -> Vec<String> {
    [
        "All/Everything",
        "Ports",
        "Public listeners",
        "Conflicts",
        "Orphans",
        "Tracked runs",
        "Projects",
        "Docker/Compose",
        "Podman",
        "systemd:user",
        if show_system {
            "systemd:system"
        } else {
            "systemd:system [hidden]"
        },
        "Direct processes",
        "Logs",
        "Doctor",
        "Process tree",
        "Metrics",
    ]
    .iter()
    .map(|s| s.to_string())
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
        "process-tree",
        "metrics",
        "theme default-dark",
        "theme high-contrast",
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
            view: ViewKind::Everything,
            active_pane: Pane::Rows,
            keybindings: None,
            selected_row: 0,
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

fn title_for_view(view: ViewKind) -> &'static str {
    match view {
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

fn group_is_active(group: &str, view: ViewKind) -> bool {
    matches!(
        (group, view),
        ("All/Everything", ViewKind::Everything)
            | ("Ports", ViewKind::Ports)
            | ("Public listeners", ViewKind::Public)
            | ("Conflicts", ViewKind::Conflicts)
            | ("Orphans", ViewKind::Orphans)
            | ("Tracked runs", ViewKind::TrackedRuns)
            | ("Projects", ViewKind::Projects)
            | ("Logs", ViewKind::Logs)
            | ("Doctor", ViewKind::Doctor)
            | ("Process tree", ViewKind::ProcessTree)
            | ("Metrics", ViewKind::Metrics)
    )
}

fn render_header(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    view: ViewKind,
) {
    let total = view_model.rows.len();
    let public = view_model
        .rows
        .iter()
        .filter(|row| row.badges.iter().any(|badge| badge == "PUBLIC"))
        .count();
    let adapters = view_model.metrics.adapters.len();
    let line = Line::from(vec![
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
        Span::styled(
            format!("{total} listeners"),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            format!("  {public} public"),
            Style::default()
                .fg(if public > 0 {
                    theme.warning.color()
                } else {
                    theme.footer.color()
                })
                .bg(theme.base_bg.color()),
        ),
        Span::styled(
            format!("  {adapters} adapters"),
            Style::default()
                .fg(theme.info.color())
                .bg(theme.base_bg.color()),
        ),
    ]);
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
    if view_model.layout == LayoutMode::Refuse {
        let p = Paragraph::new("lazyadmin TUI needs 60+ columns. Try `lazyadmin ps --json`, `lazyadmin public`, or widen the terminal.")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.base_fg.color()).bg(theme.base_bg.color()))
            .block(panel_block("lazyadmin", theme, true));
        frame.render_widget(p, area);
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
                Constraint::Length(26),
                Constraint::Min(48),
                Constraint::Length(40),
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
                    let marker = if active { "› " } else { "  " };
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            marker,
                            Style::default()
                                .fg(theme.accent.color())
                                .bg(theme.base_bg.color()),
                        ),
                        Span::styled(
                            g.clone(),
                            Style::default()
                                .fg(if active {
                                    theme.base_fg.color()
                                } else {
                                    theme.footer.color()
                                })
                                .bg(theme.base_bg.color())
                                .add_modifier(if active {
                                    Modifier::BOLD
                                } else {
                                    Modifier::empty()
                                }),
                        ),
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
    let mut status = Vec::new();
    if view_model.hidden_system_count > 0 {
        status.push(format!(
            "hidden: {} system services. press S to toggle",
            view_model.hidden_system_count
        ));
    }
    if view_model.events_dropped > 0 {
        status.push("EVENTS DROPPED — refresh may lag".into());
    }
    if let Some(degraded) = &view_model.degraded {
        status.push(format!("DEGRADED {degraded}"));
    }
    if status.is_empty() {
        status.push("lazyadmin ready — ? help, : palette".into());
    }
    frame.render_widget(
        Paragraph::new(status.join(" │ ")).style(
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ),
        vertical[2],
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
        ViewKind::ProcessTree => render_process_tree(view_model, frame, area, theme, active),
        ViewKind::Metrics => render_metrics(view_model, frame, area, theme, active),
        ViewKind::Logs => render_logs(view_model, frame, area, theme, active),
        ViewKind::Doctor => render_doctor_view(view_model, frame, area, theme, active),
        _ => render_rows_table(
            view_model,
            frame,
            area,
            theme,
            ctx.view,
            ctx.selected_row,
            active,
        ),
    }
    if let Some(keybindings) = ctx.keybindings {
        let _ = help_lines(keybindings);
    }
}

fn render_rows_table(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    view: ViewKind,
    selected_row: usize,
    active: bool,
) {
    let rows = view_model
        .rows
        .iter()
        .filter(|r| match view {
            ViewKind::Public => r.badges.iter().any(|b| b == "PUBLIC"),
            ViewKind::Ports => r.port.is_some(),
            _ => true,
        })
        .enumerate()
        .map(|(idx, r)| {
            let quiet = if idx % 2 == 0 {
                theme.base_fg.color()
            } else {
                theme.footer.color()
            };
            let exposure_style = if r.badges.iter().any(|badge| badge == "PUBLIC") {
                Style::default()
                    .fg(theme.warning.color())
                    .bg(theme.base_bg.color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.footer.color())
                    .bg(theme.base_bg.color())
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    r.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                    Style::default()
                        .fg(theme.accent.color())
                        .bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    compact_text(&r.bind, 22),
                    Style::default().fg(quiet).bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    r.owner.clone(),
                    Style::default()
                        .fg(theme.base_fg.color())
                        .bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(
                    r.runtime.clone(),
                    Style::default()
                        .fg(theme.info.color())
                        .bg(theme.base_bg.color()),
                )),
                Cell::from(Span::styled(r.exposure.clone(), exposure_style)),
            ])
            .style(Style::default().bg(theme.base_bg.color()))
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(18),
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(["Port", "Bind", "Owner", "Runtime", "Scope"]).style(
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
        .filter(|r| match view {
            ViewKind::Public => r.badges.iter().any(|b| b == "PUBLIC"),
            ViewKind::Ports => r.port.is_some(),
            _ => true,
        })
        .count();
    if row_count > 0 {
        state.select(Some(selected_row.min(row_count.saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_inspector(
    view_model: &ViewModel,
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: &Theme,
    active: bool,
) {
    let mut lines: Vec<Line<'_>> = view_model
        .inspector
        .lines
        .iter()
        .map(|line| inspector_line(line, theme))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Provenance",
        Style::default()
            .fg(theme.accent.color())
            .bg(theme.base_bg.color())
            .add_modifier(Modifier::BOLD),
    )));
    lines.extend(view_model.inspector.provenance.iter().map(|p| {
        Line::from(Span::styled(
            format!("  {}", compact_text(p, 72)),
            Style::default()
                .fg(theme.footer.color())
                .bg(theme.base_bg.color()),
        ))
    }));
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
) {
    let lines = view_model
        .rows
        .iter()
        .flat_map(|r| r.badges.iter())
        .map(|b| {
            Line::from(Span::styled(
                b.clone(),
                Style::default().fg(theme.warning.color()),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("Doctor", theme, active))
            .style(Style::default().bg(theme.base_bg.color())),
        area,
    );
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
        Row::new(vec![
            Cell::from(Span::styled(
                format!("{}{} {}", "  ".repeat(r.depth), marker, r.label),
                Style::default()
                    .fg(theme.base_fg.color())
                    .bg(theme.base_bg.color()),
            )),
            Cell::from(Span::styled(
                r.runtime.clone(),
                Style::default()
                    .fg(theme.info.color())
                    .bg(theme.base_bg.color()),
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
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(20),
        ],
    )
    .header(
        Row::new(["Process", "Runtime", "Workload", "Warnings"]).style(
            Style::default()
                .fg(theme.accent.color())
                .bg(theme.base_bg.color()),
        ),
    )
    .block(panel_block("Process Tree", theme, active))
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
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(5),
        ])
        .split(area);
    let total =
        (view_model.metrics.listeners_loopback + view_model.metrics.listeners_public).max(1) as u16;
    frame.render_widget(
        Gauge::default()
            .block(panel_block("Events dropped", theme, false))
            .gauge_style(Style::default().fg(theme.error.color()))
            .percent((view_model.metrics.events_dropped.min(100)) as u16),
        chunks[0],
    );
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
    let adapter_rows = view_model.metrics.adapters.iter().map(|adapter| {
        Row::new(vec![
            Cell::from(adapter.adapter.clone()),
            Cell::from(adapter.throughput.to_string()),
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
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(["Adapter", "Events/s", "Drops", "Latency"]).style(
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
    let bars = [
        Bar::default()
            .label("loopback".into())
            .value(view_model.metrics.listeners_loopback as u64),
        Bar::default()
            .label("public".into())
            .value(view_model.metrics.listeners_public as u64),
    ];
    frame.render_widget(
        BarChart::default()
            .block(panel_block(
                format!("Listeners total {total}"),
                theme,
                false,
            ))
            .data(BarGroup::default().bars(&bars))
            .bar_style(Style::default().fg(theme.ok.color())),
        chunks[3],
    );
}

pub fn help_lines(keybindings: &ResolvedKeybindings) -> Vec<String> {
    keybindings
        .bindings
        .iter()
        .map(|(a, b)| format!("{a}: {}", b.join(", ")))
        .collect()
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
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(markdown)) {
        Ok(()) => Ok(CopyDiagnosticOutcome::Clipboard),
        Err(_) => copy_diagnostic_via_command(markdown)
            .map(|()| CopyDiagnosticOutcome::Clipboard)
            .or_else(|_| copy_diagnostic_fallback(markdown, None).map(CopyDiagnosticOutcome::File)),
    }
}

fn copy_diagnostic_via_command(markdown: &str) -> anyhow::Result<()> {
    for program in ["wl-copy", "xclip"] {
        let mut child = std::process::Command::new(program)
            .args(if program == "xclip" {
                vec!["-selection", "clipboard"]
            } else {
                Vec::new()
            })
            .stdin(std::process::Stdio::piped())
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
    let mut app = App {
        vm: build_view_model(&initial_snapshot, w, runtime.config.show_system, ""),
        snapshot: initial_snapshot,
        show_system: runtime.config.show_system,
        theme: runtime.theme,
        keybindings: runtime.keybindings,
        status: runtime.color_hint,
        allow_open_non_loopback: runtime.allow_open_non_loopback,
        config_reload: runtime.config_reload,
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
    );
    sync_row_selection(app);
}

fn visible_row_indices(app: &App) -> Vec<usize> {
    app.vm
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| match app.active_view {
            ViewKind::Public => row.badges.iter().any(|badge| badge == "PUBLIC"),
            ViewKind::Ports => row.port.is_some(),
            ViewKind::ProcessTree | ViewKind::Metrics | ViewKind::Logs | ViewKind::Doctor => false,
            _ => true,
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn sync_row_selection(app: &mut App) {
    let visible = visible_row_indices(app);
    if visible.is_empty() {
        app.selected_row = 0;
        if app.selected_process.is_none() {
            app.vm.inspector = InspectorVm {
                title: "No selection".into(),
                lines: vec!["No workloads/listeners discovered yet".into()],
                provenance: vec![],
                provenance_expanded: false,
                diagnostic_markdown: "# lazyadmin diagnostic\nNo selection\n".into(),
            };
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
            app.vm.inspector = inspector_for_row(row);
        }
    }
}

fn scroll_rows(app: &mut App, delta: isize) {
    let visible_len = visible_row_indices(app).len();
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

fn selected_row(app: &App) -> Option<&RowVm> {
    visible_row_indices(app)
        .get(app.selected_row)
        .and_then(|idx| app.vm.rows.get(*idx))
}

fn navigable_views() -> &'static [ViewKind] {
    &[
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
    ]
}

fn active_view_index(view: ViewKind) -> usize {
    navigable_views()
        .iter()
        .position(|candidate| *candidate == view)
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
    if view == ViewKind::ProcessTree {
        if app.selected_process.is_none() {
            app.selected_process = app.vm.process_tree.rows.first().map(|row| row.key.clone());
        }
    } else {
        app.selected_process = None;
    }
    app.active_view = view;
    app.selected_row = 0;
    rebuild_view_model(app, width);
    app.status = Some(format!("view: {}", title_for_view(view)));
}

fn focus_pane(app: &mut App, pane: Pane) {
    app.pane = pane;
    app.status = Some(format!(
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
    if matches!(app.mode, InputMode::Filter) {
        match key.code {
            KeyCode::Esc => {
                app.query.clear();
                app.mode = InputMode::Normal;
                app.status = Some("filter cleared".into());
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
            app.selected_row = visible_row_indices(app).len().saturating_sub(1);
            app.selected_process = None;
            sync_row_selection(app);
            return;
        }
        _ => {}
    }
    if let Some(cmd) = key_to_command_with_bindings(key, &app.keybindings) {
        match cmd {
            Command::Quit => app.should_quit = true,
            Command::ToggleSystem => {
                app.show_system = !app.show_system;
                rebuild_view_model(app, width);
            }
            Command::Filter => app.mode = InputMode::Filter,
            Command::Palette => {
                app.query.clear();
                app.mode = InputMode::Palette;
            }
            Command::Refresh => rebuild_view_model(app, width),
            Command::NextPane => cycle_pane(app, 1, width),
            Command::PrevPane => cycle_pane(app, -1, width),
            Command::Tree => {
                if app.active_view == ViewKind::ProcessTree {
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
            Command::Help => app.mode = InputMode::Help,
            Command::CopyDiagnostic => match copy_diagnostic(&app.vm.inspector.diagnostic_markdown)
            {
                Ok(CopyDiagnosticOutcome::Clipboard) => {
                    app.status = Some("diagnostic copied".into());
                }
                Ok(CopyDiagnosticOutcome::File(path)) => {
                    app.status = Some(format!("clipboard unavailable; wrote {}", path.display()));
                }
                Err(err) => app.status = Some(format!("copy failed: {err}")),
            },
            Command::Open => match selected_row(app) {
                Some(row) => match open_row_url(row, app.allow_open_non_loopback) {
                    Ok(url) => app.status = Some(format!("opened {url}")),
                    Err(err) => app.status = Some(format!("open failed: {err}")),
                },
                None => app.status = Some("open failed: no selected listener".into()),
            },
            Command::Kill => {
                app.confirmation = Some(Confirmation {
                    command: cmd,
                    typed: String::new(),
                    required: "kill".into(),
                })
            }
            _ => CommandDispatcher::execute(&cmd),
        }
    }
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
                        app.status = Some("config reloaded".into());
                    }
                    Err(err) => app.status = Some(format!("reload failed: {err}")),
                }
            } else {
                app.status = Some("reload unavailable in this runtime".into());
            }
        }
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
                    app.status = Some(format!("theme {name} applied"));
                }
                Err(err) => app.status = Some(format!("theme failed: {err}")),
            }
        }
        "" => {}
        other => app.status = Some(format!("unknown command: {other}")),
    }
}
fn render_app(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();
    render_view_kind(
        &app.vm,
        f,
        area,
        &app.theme,
        RenderContext {
            view: app.active_view,
            active_pane: app.pane,
            keybindings: Some(&app.keybindings),
            selected_row: app.selected_row,
        },
    );
    let footer = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let status = match app.mode {
        InputMode::Filter => format!("Filter: {}  (Enter apply, Esc clear)", app.query),
        InputMode::Palette => format!("Command: {}  (Enter run, Esc cancel)", app.query),
        _ => app.status.clone().unwrap_or_default(),
    };
    if !status.is_empty() {
        f.render_widget(
            Paragraph::new(status).style(Style::default().fg(app.theme.footer.color())),
            footer,
        );
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
                Constraint::Length(24),
                Constraint::Min(40),
                Constraint::Length(32),
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
        AddressFamily, Confidence, DualStackState, Listener, ListenerId, ListenerState, Protocol,
        WarningSeverity,
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
        let mut app = App::default();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.confirmation.as_ref().unwrap().required, "kill");
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
                assert!(text.contains("60+ columns"));
            } else {
                assert!(text.contains("Everything") || text.contains("Views"));
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
    fn focused_views_pane_arrow_keys_change_active_view() {
        let snap = build_empty_snapshot();
        let mut app = App {
            vm: build_view_model(&snap, 120, false, ""),
            snapshot: snap,
            pane: Pane::Groups,
            ..Default::default()
        };

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Ports);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            120,
        );
        assert_eq!(app.active_view, ViewKind::Everything);
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

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            70,
        );
        assert_eq!(app.active_view, ViewKind::Ports);

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            70,
        );
        assert_eq!(app.active_view, ViewKind::Everything);
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

    #[test]
    fn theme_builtins_validate_and_downgrade() {
        for name in [
            "default-dark",
            "default-light",
            "high-contrast",
            "solarized-dark",
        ] {
            let mut theme = Theme::builtin(name).unwrap();
            theme.validate().unwrap();
            assert_eq!(theme.name, name);
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
