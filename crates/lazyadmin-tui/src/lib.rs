#![forbid(unsafe_code)]

use std::{
    io, panic,
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
use lazyadmin_core::{model::Snapshot, snapshot::build_empty_snapshot};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tracing::{debug, info, info_span};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub refresh_interval: Duration,
    pub show_system: bool,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            refresh_interval: Duration::from_secs(2),
            show_system: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct App {
    pub vm: ViewModel,
    pub pane: Pane,
    pub query: String,
    pub mode: InputMode,
    pub should_quit: bool,
    pub show_system: bool,
    pub confirmation: Option<Confirmation>,
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

pub struct EventLoop {
    pub rx: mpsc::Receiver<UiEvent>,
}
#[derive(Debug)]
pub enum UiEvent {
    Input(KeyEvent),
    Snapshot(Box<Snapshot>),
    Tick,
}

pub struct SnapshotController {
    tx: watch::Sender<Snapshot>,
    handle: JoinHandle<()>,
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
    pub inspector: InspectorVm,
    pub hidden_system_count: usize,
    pub degraded: Option<String>,
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

pub fn build_view_model(
    snapshot: &Snapshot,
    width: u16,
    show_system: bool,
    filter: &str,
) -> ViewModel {
    let layout = match width {
        100..=u16::MAX => LayoutMode::ThreePane,
        80..=99 => LayoutMode::InspectorTab,
        60..=79 => LayoutMode::SinglePane,
        _ => LayoutMode::Refuse,
    };
    let mut rows = Vec::new();
    let mut hidden = 0usize;
    for l in &snapshot.listeners {
        let is_system = l
            .provenance
            .iter()
            .any(|p| p.claim.contains("systemd:system"));
        if is_system && !show_system {
            hidden += 1;
            continue;
        }
        let owner = l
            .owners
            .first()
            .map(|o| format!("{o:?}"))
            .unwrap_or_else(|| "unknown".into());
        let runtime = if is_system {
            "SystemdSystem".to_string()
        } else {
            "direct".into()
        };
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
            "{:?} {} {} {} {:?}",
            l.port, bind, owner, runtime, l.protocol
        );
        rows.push(RowVm {
            id: l.id.to_string(),
            port: l.port,
            bind,
            owner,
            runtime,
            project: "-".into(),
            badges,
            search_text,
        });
    }
    if !filter.is_empty() {
        let m = SkimMatcherV2::default();
        rows.retain(|r| m.fuzzy_match(&r.search_text, filter).is_some());
    }
    let inspector = rows
        .first()
        .map(inspector_for_row)
        .unwrap_or_else(|| InspectorVm {
            title: "No selection".into(),
            lines: vec!["No workloads/listeners discovered yet".into()],
            provenance: vec![],
            provenance_expanded: false,
            diagnostic_markdown: "# lazyadmin diagnostic\nNo selection\n".into(),
        });
    ViewModel {
        width,
        layout,
        groups: groups(show_system),
        rows,
        inspector,
        hidden_system_count: hidden,
        degraded: None,
    }
}
fn inspector_for_row(row: &RowVm) -> InspectorVm {
    InspectorVm {
        title: row.owner.clone(),
        lines: vec![
            format!("identity: {}", row.id),
            format!("state: unknown"),
            format!("runtime: {}", row.runtime),
            format!("ports/listeners: {:?}", row.port),
            format!("project: {}", row.project),
            "logs: no log source for raw direct processes unless manager metadata is available"
                .into(),
            format!("warnings: {}", row.badges.join(", ")),
            "actions: open logs restart stop free-port copy-diagnostic".into(),
        ],
        provenance: vec![
            "▶ listener discovered via core snapshot services".into(),
            "confidence: best-effort".into(),
        ],
        provenance_expanded: false,
        diagnostic_markdown: format!(
            "# lazyadmin diagnostic\n\n- owner: {}\n- port: {:?}\n- runtime: {}\n- provenance: core snapshot\n",
            row.owner, row.port, row.runtime
        ),
    }
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
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn key_to_command(key: KeyEvent) -> Option<Command> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('/'), _) => Some(Command::Filter),
        (KeyCode::Char(':'), _) => Some(Command::Palette),
        (KeyCode::Tab, _) => Some(Command::NextPane),
        (KeyCode::BackTab, _) => Some(Command::PrevPane),
        (KeyCode::Enter, _) => Some(Command::Inspect),
        (KeyCode::Char('l'), _) => Some(Command::Logs),
        (KeyCode::Char('p'), _) => Some(Command::Ports),
        (KeyCode::Char('t'), _) => Some(Command::Tree),
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

pub async fn run_tui(snapshot: Snapshot) -> Result<()> {
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
    let mut app = App {
        vm: build_view_model(&snapshot, w, false, ""),
        ..Default::default()
    };
    let started = Instant::now();
    loop {
        let render_started = Instant::now();
        terminal.draw(|f| render(f, &app))?;
        debug!(
            elapsed_ms = render_started.elapsed().as_millis(),
            "tui.render"
        );
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                let input_started = Instant::now();
                handle_key(&mut app, key);
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

fn handle_key(app: &mut App, key: KeyEvent) {
    if let Some(cmd) = key_to_command(key) {
        match cmd {
            Command::Quit => app.should_quit = true,
            Command::ToggleSystem => {
                app.show_system = !app.show_system;
            }
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
fn render(f: &mut ratatui::Frame<'_>, app: &App) {
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
}
