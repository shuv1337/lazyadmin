use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub ui: UiConfig,
    pub ports: PortsConfig,
    pub actions: ActionsConfig,
    pub redaction: RedactionConfig,
    pub adapters: AdaptersConfig,
    pub projects: ProjectsConfig,
    pub visibility: VisibilityConfig,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    pub refresh_interval_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsConfig {
    pub common: Vec<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionsConfig {
    pub require_confirmation: bool,
    pub free_multi_owner: FreeMultiOwner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreeMultiOwner {
    StopAll,
    Prompt,
    Refuse,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionConfig {
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptersConfig {
    pub sockets: AdapterToggle,
    pub systemd: AdapterToggle,
    pub container: AdapterToggle,
    pub tracked: AdapterToggle,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterToggle {
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsConfig {
    pub roots: Vec<PathBuf>,
    pub markers: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityConfig {
    pub system_service_denylist: SystemServiceDenylist,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemServiceDenylist {
    pub units: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig {
                refresh_interval_ms: 1000,
            },
            ports: PortsConfig {
                common: vec![3000, 5173, 5432, 6379, 8080],
            },
            actions: ActionsConfig {
                require_confirmation: true,
                free_multi_owner: FreeMultiOwner::StopAll,
            },
            redaction: RedactionConfig { enabled: true },
            adapters: AdaptersConfig {
                sockets: AdapterToggle { enabled: true },
                systemd: AdapterToggle { enabled: true },
                container: AdapterToggle { enabled: true },
                tracked: AdapterToggle { enabled: true },
            },
            projects: ProjectsConfig {
                roots: vec![
                    PathBuf::from("~/src"),
                    PathBuf::from("~/code"),
                    PathBuf::from("~/work"),
                ],
                markers: vec![
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
                .collect(),
            },
            visibility: VisibilityConfig {
                system_service_denylist: SystemServiceDenylist {
                    units: vec![
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
                    .collect(),
                },
            },
        }
    }
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
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (100..=60_000).contains(&self.ui.refresh_interval_ms),
            "ui.refresh_interval_ms must be between 100 and 60000"
        );
        let mut seen = HashSet::new();
        for p in &self.projects.roots {
            let key = p.to_string_lossy().to_string();
            anyhow::ensure!(seen.insert(key), "duplicate project root: {}", p.display());
        }
        Ok(())
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
}
