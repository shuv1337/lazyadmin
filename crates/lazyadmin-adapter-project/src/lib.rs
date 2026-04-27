#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
use async_trait::async_trait;
use chrono::Utc;
use lazyadmin_core::{
    config::Config,
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug)]
pub struct ProjectAdapter {
    config: Config,
    cache: HashMap<PathBuf, Option<Project>>,
}
impl ProjectAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            cache: HashMap::new(),
        }
    }
    pub fn markers() -> Vec<&'static str> {
        vec![
            ".git",
            "package.json",
            "bun.lock",
            "pnpm-lock.yaml",
            "yarn.lock",
            "package-lock.json",
            "pyproject.toml",
            "uv.lock",
            "requirements.txt",
            "Cargo.toml",
            "go.mod",
            "compose.yaml",
            "compose.yml",
            "docker-compose.yaml",
            "docker-compose.yml",
            "flake.nix",
            "devbox.json",
            ".envrc",
            "Procfile",
            "Makefile",
        ]
    }
    pub fn detect_path(&mut self, path: &Path, confidence: Confidence) -> Option<Project> {
        let start = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        for (k, v) in &self.cache {
            if start.starts_with(k) {
                return v.clone();
            }
        }
        let mut cur = Some(start);
        while let Some(dir) = cur {
            let markers: Vec<_> = Self::markers()
                .into_iter()
                .filter_map(|m| {
                    let p = dir.join(m);
                    p.exists().then(|| ProjectMarker {
                        kind: m.into(),
                        path: p,
                    })
                })
                .collect();
            if !markers.is_empty() {
                let pr = project(dir.to_path_buf(), markers, confidence);
                self.cache.insert(start.to_path_buf(), Some(pr.clone()));
                return Some(pr);
            }
            if self.config.projects.roots.iter().any(|r| dir == r) {
                break;
            }
            cur = dir.parent();
        }
        self.cache.insert(start.to_path_buf(), None);
        None
    }
}
fn prov(claim: &str, e: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "project".into(),
        claim: claim.into(),
        evidence: e.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
fn project(root: PathBuf, markers: Vec<ProjectMarker>, confidence: Confidence) -> Project {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .into();
    let pm = if markers.iter().any(|m| m.kind == "pnpm-lock.yaml") {
        Some("pnpm".into())
    } else if markers.iter().any(|m| m.kind == "bun.lock") {
        Some("bun".into())
    } else if markers.iter().any(|m| m.kind == "Cargo.toml") {
        Some("cargo".into())
    } else if markers.iter().any(|m| m.kind == "package.json") {
        Some("npm".into())
    } else {
        None
    };
    Project {
        id: ProjectId::new(root.to_string_lossy()),
        root,
        name,
        markers,
        git_remote: None,
        package_manager: pm,
        dev_commands: vec![],
        provenance: vec![prov(
            "marker project root",
            "upward marker scan",
            confidence,
        )],
    }
}
#[async_trait]
impl DiscoveryAdapter for ProjectAdapter {
    fn name(&self) -> &'static str {
        "project"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }
    async fn health(&self) -> AdapterHealth {
        AdapterHealth {
            adapter: "project".into(),
            available: true,
            message: Some("marker-based project detection".into()),
        }
    }
    #[tracing::instrument(name = "adapter.project.detect", skip_all)]
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let start = Instant::now();
        let mut out = DiscoveryOutput::default();
        for r in &self.config.projects.roots {
            if r.exists() {
                let mut ad = self.clone();
                if let Some(p) = ad.detect_path(r, Confidence::Low) {
                    out.projects.push(p)
                }
            }
        }
        tracing::debug!(
            candidate_count = self.config.projects.roots.len(),
            markers_found = out.projects.len(),
            duration_ms = start.elapsed().as_millis(),
            "project detect"
        );
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn rust_marker() {
        let d = std::env::temp_dir().join(format!("lazyadmin-proj-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("crates/x")).unwrap();
        fs::write(d.join("Cargo.toml"), "").unwrap();
        let mut a = ProjectAdapter::new(Config::default());
        let p = a
            .detect_path(&d.join("crates/x"), Confidence::High)
            .unwrap();
        assert_eq!(p.package_manager.as_deref(), Some("cargo"));
        let _ = fs::remove_dir_all(&d);
    }
    #[test]
    fn no_marker() {
        let d = std::env::temp_dir().join(format!("lazyadmin-nomarker-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let mut a = ProjectAdapter::new(Config::default());
        assert!(a.detect_path(&d, Confidence::High).is_none());
        let _ = fs::remove_dir_all(&d);
    }
}
