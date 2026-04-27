#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
use async_trait::async_trait;
use chrono::Utc;
use lazyadmin_core::{
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct TrackedAdapter {
    registry: Registry,
}
#[derive(Clone, Debug)]
pub struct Registry {
    root: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunEntry {
    pub id: String,
    pub tag: Option<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env_hash: Option<String>,
    pub started_at: chrono::DateTime<Utc>,
    pub creator: String,
    pub scope_or_unit_name: Option<String>,
    pub state: WorkloadState,
    pub log_source: String,
    pub spawn_method: String,
    pub pid: Option<u32>,
    pub log_file: Option<PathBuf>,
}
fn prov(claim: &str, evidence: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "tracked".into(),
        claim: claim.into(),
        evidence: evidence.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            root: runtime_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("lazyadmin/runs"),
        }
    }
}
fn runtime_dir() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}
impl Registry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn ensure(&self) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::set_permissions(
            self.root.parent().unwrap_or(&self.root),
            fs::Permissions::from_mode(0o700),
        )
        .ok();
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).ok();
        Ok(())
    }
    pub fn list(&self) -> io::Result<Vec<RunEntry>> {
        self.ensure()?;
        let mut v = Vec::new();
        for e in fs::read_dir(&self.root)? {
            let e = e?;
            if e.path().extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(text) = fs::read_to_string(e.path()) {
                    if let Ok(mut r) = serde_json::from_str::<RunEntry>(&text) {
                        reconcile_entry(&mut r);
                        v.push(r)
                    } else {
                        let _ = fs::rename(e.path(), e.path().with_extension("json.bad"));
                    }
                }
            }
        }
        Ok(v)
    }
    pub fn save(&self, entry: &RunEntry) -> io::Result<()> {
        self.ensure()?;
        let path = self.root.join(format!("{}.json", entry.id));
        fs::write(path, serde_json::to_vec_pretty(entry)?)
    }
    pub fn resolve(&self, sel: &str) -> io::Result<Option<RunEntry>> {
        let key = sel.strip_prefix("tag:").unwrap_or(sel);
        Ok(self
            .list()?
            .into_iter()
            .find(|r| r.id == key || r.tag.as_deref() == Some(key)))
    }
    pub fn forget(&self, sel: &str) -> io::Result<bool> {
        if let Some(r) = self.resolve(sel)? {
            fs::remove_file(self.root.join(format!("{}.json", r.id))).ok();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
fn reconcile_entry(r: &mut RunEntry) {
    if r.state == WorkloadState::Running {
        if let Some(pid) = r.pid {
            if !Path::new("/proc").join(pid.to_string()).exists() {
                r.state = WorkloadState::Exited;
            }
        }
    }
}
impl TrackedAdapter {
    pub fn new() -> Self {
        Self {
            registry: Registry::default(),
        }
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}
impl Default for TrackedAdapter {
    fn default() -> Self {
        Self::new()
    }
}
#[async_trait]
impl DiscoveryAdapter for TrackedAdapter {
    fn name(&self) -> &'static str {
        "tracked"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }
    async fn health(&self) -> AdapterHealth {
        AdapterHealth {
            adapter: "tracked".into(),
            available: runtime_dir().is_some(),
            message: Some(format!(
                "runtime_dir_present={} registry_dir={}",
                runtime_dir().is_some(),
                self.registry.path().display()
            )),
        }
    }
    #[tracing::instrument(name = "adapter.tracked.discover", skip_all)]
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let mut out = DiscoveryOutput::default();
        for r in self.registry.list()? {
            let id = RunId::new(r.id.clone());
            out.tracked_runs.push(TrackedRun {
                id: id.clone(),
                tag: r.tag.clone(),
                command: r.cmd.clone(),
                cwd: r.cwd.clone(),
                state: r.state.clone(),
                started_at: Some(r.started_at),
                provenance: vec![prov(
                    "registry entry",
                    self.registry.path().display().to_string(),
                    Confidence::High,
                )],
            });
            out.workloads.push(Workload {
                id: WorkloadId::new(format!("tracked:{}", r.id)),
                display_name: r.tag.clone().unwrap_or(r.id.clone()),
                runtime: RuntimeKind::LazyadminTracked,
                state: r.state,
                pids: vec![],
                listeners: vec![],
                project: None,
                manager: None,
                source: Some(EntityRef::Run(id)),
                actions: vec![],
                health: None,
                metrics: None,
                restart_policy: None,
                lazyadmin_run_id: Some(RunId::new(r.id)),
                provenance: vec![prov("tracked workload", "registry", Confidence::High)],
            });
        }
        Ok(out)
    }
}

pub fn spawn_detached(
    tag: Option<String>,
    cwd: Option<PathBuf>,
    envs: Vec<(String, String)>,
    cmd: Vec<String>,
) -> anyhow::Result<RunEntry> {
    anyhow::ensure!(!cmd.is_empty(), "command required");
    let reg = Registry::default();
    let short = Uuid::now_v7().to_string()[..8].to_string();
    let id = if let Some(t) = &tag {
        if reg.resolve(&format!("tag:{t}"))?.is_some() {
            format!("{t}-{short}")
        } else {
            t.clone()
        }
    } else {
        format!("run-{short}")
    };
    reg.ensure()?;
    let log_file = runtime_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lazyadmin/logs")
        .join(format!("{id}.log"));
    fs::create_dir_all(log_file.parent().unwrap())?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;
    let log2 = log.try_clone()?;
    let mut c = Command::new(&cmd[0]);
    c.args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2));
    if let Some(cwd) = &cwd {
        c.current_dir(cwd);
    }
    for (k, v) in envs {
        c.env(k, v);
    }
    let child = c.spawn()?;
    let entry = RunEntry {
        id: id.clone(),
        tag,
        cmd,
        cwd,
        env_hash: None,
        started_at: Utc::now(),
        creator: env::var("USER").unwrap_or_else(|_| "unknown".into()),
        scope_or_unit_name: None,
        state: WorkloadState::Running,
        log_source: "file".into(),
        spawn_method: "direct_detached_file_log".into(),
        pid: Some(child.id()),
        log_file: Some(log_file),
    };
    reg.save(&entry)?;
    Ok(entry)
}
pub fn stop(sel: &str) -> anyhow::Result<bool> {
    let reg = Registry::default();
    if let Some(mut r) = reg.resolve(sel)? {
        if let Some(pid) = r.pid {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(format!("-{pid}"))
                .status();
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
        }
        r.state = WorkloadState::Exited;
        reg.save(&r)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
pub fn logs(sel: &str) -> anyhow::Result<String> {
    let reg = Registry::default();
    let r = reg
        .resolve(sel)?
        .ok_or_else(|| anyhow::anyhow!("run not found"))?;
    Ok(r.log_file
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default())
}
pub fn forget(sel: &str) -> anyhow::Result<bool> {
    Ok(Registry::default().forget(sel)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_path() {
        let r = Registry::new(PathBuf::from("/tmp/lazyadmin-test-runs"));
        assert!(r.path().ends_with("lazyadmin-test-runs"));
    }
}
