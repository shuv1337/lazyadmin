use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub ports: PortsConfig,
    #[serde(default)]
    pub actions: ActionsConfig,
    #[serde(default)]
    pub redaction: RedactionConfig,
    #[serde(default)]
    pub adapters: AdaptersConfig,
    #[serde(default)]
    pub projects: ProjectsConfig,
    #[serde(default)]
    pub visibility: VisibilityConfig,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_legacy_refresh_interval_ms")]
    pub refresh_interval_ms: u64,
    #[serde(default)]
    pub theme: UiThemeConfig,
    #[serde(default)]
    pub keybindings: UiKeybindingsConfig,
    #[serde(default)]
    pub refresh: UiRefreshConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiThemeConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiKeybindingsConfig {
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiRefreshConfig {
    #[serde(default = "default_tick_ms")]
    pub tick_ms: u64,
    #[serde(default = "default_event_debounce_ms")]
    pub event_debounce_ms: u64,
    #[serde(default = "default_max_redraw_hz")]
    pub max_redraw_hz: u64,
}

impl Default for UiRefreshConfig {
    fn default() -> Self {
        Self {
            tick_ms: default_tick_ms(),
            event_debounce_ms: default_event_debounce_ms(),
            max_redraw_hz: default_max_redraw_hz(),
        }
    }
}

fn default_tick_ms() -> u64 {
    500
}
fn default_event_debounce_ms() -> u64 {
    100
}
fn default_max_redraw_hz() -> u64 {
    30
}
fn default_legacy_refresh_interval_ms() -> u64 {
    1000
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsConfig {
    #[serde(default = "default_common_ports")]
    pub common: Vec<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionsConfig {
    #[serde(default = "default_true")]
    pub require_confirmation: bool,
    #[serde(default)]
    pub free_multi_owner: FreeMultiOwner,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreeMultiOwner {
    #[default]
    StopAll,
    Prompt,
    Refuse,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptersConfig {
    #[serde(default)]
    pub sockets: SocketsAdapterConfig,
    #[serde(default)]
    pub events: EventsConfig,
    #[serde(default)]
    pub systemd: AdapterToggle,
    #[serde(default)]
    pub container: AdapterToggle,
    #[serde(default)]
    pub tracked: AdapterToggle,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketsAdapterConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub preferred: SocketDiscoveryPreference,
    #[serde(default = "default_true")]
    pub confirm_dual_stack: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketDiscoveryPreference {
    #[default]
    Proc,
    SockDiag,
    Both,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
}
impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            channel_capacity: default_channel_capacity(),
        }
    }
}
fn default_true() -> bool {
    true
}
fn default_channel_capacity() -> usize {
    256
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterToggle {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsConfig {
    #[serde(default = "default_project_roots")]
    pub roots: Vec<PathBuf>,
    #[serde(default = "default_project_markers")]
    pub markers: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityConfig {
    #[serde(default)]
    pub system_service_denylist: SystemServiceDenylist,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemServiceDenylist {
    #[serde(default = "default_system_service_denylist_units")]
    pub units: Vec<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            refresh_interval_ms: default_legacy_refresh_interval_ms(),
            theme: UiThemeConfig::default(),
            keybindings: UiKeybindingsConfig::default(),
            refresh: UiRefreshConfig::default(),
        }
    }
}

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            common: default_common_ports(),
        }
    }
}

fn default_common_ports() -> Vec<u16> {
    vec![3000, 5173, 5432, 6379, 8080]
}

impl Default for ActionsConfig {
    fn default() -> Self {
        Self {
            require_confirmation: true,
            free_multi_owner: FreeMultiOwner::StopAll,
        }
    }
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for SocketsAdapterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            preferred: SocketDiscoveryPreference::Proc,
            confirm_dual_stack: true,
        }
    }
}

impl Default for AdapterToggle {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for ProjectsConfig {
    fn default() -> Self {
        Self {
            roots: default_project_roots(),
            markers: default_project_markers(),
        }
    }
}

fn default_project_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("~/src"),
        PathBuf::from("~/code"),
        PathBuf::from("~/work"),
    ]
}

fn default_project_markers() -> Vec<String> {
    vec![
        ".git",
        "package.json",
        "pyproject.toml",
        "Cargo.toml",
        "go.mod",
        "compose.yaml",
        "flake.nix",
        ".envrc",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for SystemServiceDenylist {
    fn default() -> Self {
        Self {
            units: default_system_service_denylist_units(),
        }
    }
}

fn default_system_service_denylist_units() -> Vec<String> {
    vec![
        "systemd-resolved.service",
        "systemd-networkd.service",
        "systemd-timesyncd.service",
        "systemd-logind.service",
        "systemd-udevd.service",
        "NetworkManager.service",
        "dbus.service",
        "avahi-daemon.service",
        "cups.service",
        "chronyd.service",
        "sshd.service",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Config {
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut cfg = Config::default();
        let owned_default;
        let chosen = if let Some(path) = path {
            Some(path)
        } else {
            owned_default = default_config_path();
            owned_default.as_deref()
        };
        if let Some(path) = chosen {
            if path.exists() {
                let text = std::fs::read_to_string(path)?;
                cfg = toml::from_str(&text)?;
            }
        }
        cfg.expand_paths();
        cfg.validate()?;
        Ok(cfg)
    }
    fn expand_paths(&mut self) {
        self.projects.roots = self.projects.roots.iter().map(|p| expand_path(p)).collect();
        if let Some(path) = self.ui.theme.path.as_deref() {
            self.ui.theme.path = Some(expand_path(path));
        }
        if let Some(path) = self.ui.keybindings.path.as_deref() {
            self.ui.keybindings.path = Some(expand_path(path));
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (100..=60_000).contains(&self.ui.refresh_interval_ms),
            "ui.refresh_interval_ms must be between 100 and 60000"
        );
        anyhow::ensure!(
            self.adapters.events.channel_capacity > 0,
            "adapters.events.channel_capacity must be greater than 0"
        );
        anyhow::ensure!(
            (50..=60_000).contains(&self.ui.refresh.tick_ms),
            "ui.refresh.tick_ms must be between 50 and 60000"
        );
        anyhow::ensure!(
            (0..=5_000).contains(&self.ui.refresh.event_debounce_ms),
            "ui.refresh.event_debounce_ms must be between 0 and 5000"
        );
        anyhow::ensure!(
            (1..=120).contains(&self.ui.refresh.max_redraw_hz),
            "ui.refresh.max_redraw_hz must be between 1 and 120"
        );
        if self.ui.theme.name.is_some() && self.ui.theme.path.is_some() {
            anyhow::bail!("only one of ui.theme.name or ui.theme.path may be set");
        }
        keybindings::ResolvedKeybindings::from_config(self)?;
        let mut seen = HashSet::new();
        for p in &self.projects.roots {
            let key = p.to_string_lossy().to_string();
            anyhow::ensure!(seen.insert(key), "duplicate project root: {}", p.display());
        }
        Ok(())
    }
}

pub mod keybindings {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum KeybindAction {
        Quit,
        Help,
        NextPane,
        PrevPane,
        OpenPalette,
        Filter,
        ToggleFilter,
        ToggleSystem,
        Inspect,
        Logs,
        Ports,
        ProcessTree,
        Metrics,
        Restart,
        Stop,
        FreePort,
        Kill,
        Open,
        Edit,
        CopyDiagnostic,
        Run,
        Refresh,
    }

    impl KeybindAction {
        pub fn as_name(self) -> &'static str {
            match self {
                Self::Quit => "quit",
                Self::Help => "help",
                Self::NextPane => "next_pane",
                Self::PrevPane => "prev_pane",
                Self::OpenPalette => "open_palette",
                Self::Filter => "filter",
                Self::ToggleFilter => "toggle_filter",
                Self::ToggleSystem => "toggle_system",
                Self::Inspect => "inspect",
                Self::Logs => "logs",
                Self::Ports => "ports",
                Self::ProcessTree => "process_tree",
                Self::Metrics => "metrics",
                Self::Restart => "restart",
                Self::Stop => "stop",
                Self::FreePort => "free_port",
                Self::Kill => "kill",
                Self::Open => "open",
                Self::Edit => "edit",
                Self::CopyDiagnostic => "copy_diag",
                Self::Run => "run",
                Self::Refresh => "refresh",
            }
        }
        pub fn all() -> &'static [Self] {
            &[
                Self::Quit,
                Self::Help,
                Self::NextPane,
                Self::PrevPane,
                Self::OpenPalette,
                Self::Filter,
                Self::ToggleFilter,
                Self::ToggleSystem,
                Self::Inspect,
                Self::Logs,
                Self::Ports,
                Self::ProcessTree,
                Self::Metrics,
                Self::Restart,
                Self::Stop,
                Self::FreePort,
                Self::Kill,
                Self::Open,
                Self::Edit,
                Self::CopyDiagnostic,
                Self::Run,
                Self::Refresh,
            ]
        }
        pub fn parse(name: &str) -> Option<Self> {
            Self::all().iter().copied().find(|a| a.as_name() == name)
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct KeybindingsFile {
        #[serde(default = "default_inherit")]
        pub inherit: String,
        #[serde(default)]
        pub overrides: BTreeMap<String, String>,
    }
    fn default_inherit() -> String {
        "default".into()
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ResolvedKeybindings {
        pub bindings: BTreeMap<String, Vec<String>>,
    }

    impl ResolvedKeybindings {
        pub fn default_map() -> BTreeMap<KeybindAction, Vec<String>> {
            use KeybindAction::*;
            BTreeMap::from([
                (Quit, vec!["q".into(), "ctrl+c".into()]),
                (Help, vec!["?".into()]),
                (NextPane, vec!["tab".into()]),
                (PrevPane, vec!["shift+tab".into()]),
                (OpenPalette, vec![":".into()]),
                (Filter, vec!["/".into()]),
                (ToggleFilter, vec![]),
                (ToggleSystem, vec!["S".into()]),
                (Inspect, vec!["enter".into()]),
                (Logs, vec!["l".into()]),
                (Ports, vec!["p".into()]),
                (ProcessTree, vec!["t".into()]),
                (Metrics, vec!["m".into()]),
                (Restart, vec!["r".into()]),
                (Stop, vec!["s".into()]),
                (FreePort, vec!["f".into()]),
                (Kill, vec!["k".into()]),
                (Open, vec!["o".into()]),
                (Edit, vec!["e".into()]),
                (CopyDiagnostic, vec!["y".into()]),
                (Run, vec!["R".into()]),
                (Refresh, vec!["F5".into()]),
            ])
        }
        pub fn from_config(cfg: &Config) -> anyhow::Result<Self> {
            let mut overrides = cfg.ui.keybindings.overrides.clone();
            if let Some(path) = &cfg.ui.keybindings.path {
                let text = std::fs::read_to_string(path)?;
                let file: KeybindingsFile = toml::from_str(&text)?;
                if file.inherit != "default" {
                    anyhow::bail!("unsupported keybindings inherit preset: {}", file.inherit);
                }
                overrides.extend(file.overrides);
            }
            let mut map = Self::default_map();
            for (name, spec) in overrides {
                let action = KeybindAction::parse(&name).ok_or_else(|| {
                    anyhow::anyhow!("unknown keybinding action `{name}`{}", suggestion(&name))
                })?;
                validate_key_spec(&spec)?;
                map.insert(action, vec![spec]);
            }
            let mut seen: HashMap<String, KeybindAction> = HashMap::new();
            for (action, specs) in &map {
                for spec in specs {
                    let norm = normalize_key(spec);
                    if let Some(prev) = seen.insert(norm.clone(), *action) {
                        if prev != *action {
                            anyhow::bail!(
                                "duplicate keybinding `{spec}` for {} and {}",
                                prev.as_name(),
                                action.as_name()
                            );
                        }
                    }
                }
            }
            Ok(Self {
                bindings: map
                    .into_iter()
                    .map(|(a, b)| (a.as_name().into(), b))
                    .collect(),
            })
        }
    }

    pub fn validate_key_spec(spec: &str) -> anyhow::Result<()> {
        let s = spec.trim();
        anyhow::ensure!(!s.is_empty(), "keybinding spec cannot be empty");
        let lower = s.to_ascii_lowercase();
        let known = [
            "tab",
            "shift+tab",
            "enter",
            "esc",
            "f5",
            "up",
            "down",
            "left",
            "right",
        ];
        if known.contains(&lower.as_str()) || lower.starts_with("ctrl+") || s.chars().count() == 1 {
            Ok(())
        } else {
            anyhow::bail!("unsupported keybinding spec `{spec}`")
        }
    }
    fn normalize_key(spec: &str) -> String {
        let trimmed = spec.trim();
        if trimmed.chars().count() == 1 {
            trimmed.to_string()
        } else {
            trimmed.to_ascii_lowercase()
        }
    }
    fn suggestion(name: &str) -> String {
        let mut best = None;
        for action in KeybindAction::all() {
            let d = levenshtein(name, action.as_name());
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((action.as_name(), d));
            }
        }
        best.filter(|(_, d)| *d <= 5)
            .map(|(s, _)| format!("; did you mean `{s}`?"))
            .unwrap_or_default()
    }
    fn levenshtein(a: &str, b: &str) -> usize {
        let mut costs: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut last = i;
            costs[0] = i + 1;
            for (j, cb) in b.chars().enumerate() {
                let old = costs[j + 1];
                costs[j + 1] = if ca == cb {
                    last
                } else {
                    1 + last.min(costs[j]).min(costs[j + 1])
                };
                last = old;
            }
        }
        *costs.last().unwrap_or(&0)
    }
}
fn default_config_path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("lazyadmin/config.toml"))
}
fn expand_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let mut out = if let Some(rest) = s.strip_prefix("~/") {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest)
    } else if s == "~" {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
    } else {
        PathBuf::from(s.as_ref())
    };
    for var in ["XDG_STATE_HOME", "XDG_RUNTIME_DIR"] {
        if out.to_string_lossy().contains(&format!("${var}")) {
            if let Ok(v) = env::var(var) {
                out = PathBuf::from(out.to_string_lossy().replace(&format!("${var}"), &v));
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_validate() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn keybindings_default_and_overrides_validate() {
        let cfg = Config::default();
        let resolved = keybindings::ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(resolved.bindings["quit"], vec!["q", "ctrl+c"]);
        let mut cfg = Config::default();
        cfg.ui
            .keybindings
            .overrides
            .insert("quit".into(), "Q".into());
        let resolved = keybindings::ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(resolved.bindings["quit"], vec!["Q"]);
    }

    #[test]
    fn keybindings_reject_duplicate_and_unknown() {
        let mut cfg = Config::default();
        cfg.ui
            .keybindings
            .overrides
            .insert("quit".into(), "o".into());
        let err = keybindings::ResolvedKeybindings::from_config(&cfg)
            .unwrap_err()
            .to_string();
        assert!(err.contains("duplicate keybinding"));
        let mut cfg = Config::default();
        cfg.ui
            .keybindings
            .overrides
            .insert("quite".into(), "Q".into());
        let err = keybindings::ResolvedKeybindings::from_config(&cfg)
            .unwrap_err()
            .to_string();
        assert!(err.contains("did you mean `quit`"));
    }

    #[test]
    fn partial_config_merges_with_defaults() {
        let path = std::env::temp_dir().join(format!(
            "lazyadmin-partial-config-{}.toml",
            uuid::Uuid::now_v7()
        ));
        std::fs::write(
            &path,
            r#"
[ui.keybindings.overrides]
help = "esc"
"#,
        )
        .unwrap();

        let cfg = Config::load(Some(&path)).unwrap();
        let resolved = keybindings::ResolvedKeybindings::from_config(&cfg).unwrap();
        assert_eq!(cfg.ui.refresh_interval_ms, 1000);
        assert_eq!(cfg.adapters.events.channel_capacity, 256);
        assert_eq!(resolved.bindings["help"], vec!["esc"]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn partial_nested_adapter_config_merges_with_defaults() {
        let path = std::env::temp_dir().join(format!(
            "lazyadmin-partial-adapter-config-{}.toml",
            uuid::Uuid::now_v7()
        ));
        std::fs::write(
            &path,
            r#"
[adapters.sockets]
preferred = "both"
"#,
        )
        .unwrap();

        let cfg = Config::load(Some(&path)).unwrap();
        assert!(cfg.adapters.sockets.enabled);
        assert_eq!(
            cfg.adapters.sockets.preferred,
            SocketDiscoveryPreference::Both
        );
        assert!(cfg.adapters.systemd.enabled);
        assert!(cfg.adapters.container.enabled);

        let _ = std::fs::remove_file(path);
    }
}
