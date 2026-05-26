#![forbid(unsafe_code)]

use std::path::Path;

use lazyadmin_core::{
    config::Config,
    correlate::EventDropCounter,
    graph::{DiscoveryAdapter, DiscoveryContext},
    model::{DiscoveryEvent, RunId, Snapshot},
    snapshot::SnapshotBuilder,
};

pub mod live;
pub mod view_model;

pub use live::{LiveSnapshotFeed, LiveSnapshotFeedSettings, spawn_live_snapshot_feed};

pub type Result<T> = anyhow::Result<T>;

pub async fn load_config(config_path: Option<&Path>) -> Result<Config> {
    Config::load(config_path)
}

pub async fn build_snapshot(config_path: Option<&Path>) -> Result<Snapshot> {
    build_snapshot_with_event_drops(config_path, None).await
}

pub async fn build_snapshot_with_event_drops(
    config_path: Option<&Path>,
    event_drops: Option<&EventDropCounter>,
) -> Result<Snapshot> {
    let cfg = load_config(config_path).await?;
    build_snapshot_from_config(&cfg, event_drops).await
}

pub async fn build_snapshot_from_config(
    cfg: &Config,
    event_drops: Option<&EventDropCounter>,
) -> Result<Snapshot> {
    let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg.clone());
    let tracked = lazyadmin_adapter_tracked::TrackedAdapter::new();
    let systemd =
        lazyadmin_adapter_systemd::SystemdAdapter::new(cfg.adapters.systemd.events_enabled);
    let container = lazyadmin_adapter_container::ContainerAdapter::new();
    let project = lazyadmin_adapter_project::ProjectAdapter::new(cfg.clone());
    let portless = lazyadmin_adapter_portless::PortlessAdapter::new();
    let mut outputs = Vec::new();
    outputs.push(procfs.discover(DiscoveryContext::default()).await?);
    outputs.push(tracked.discover(DiscoveryContext::default()).await?);
    outputs.push(systemd.discover(DiscoveryContext::default()).await?);
    outputs.push(container.discover(DiscoveryContext::default()).await?);
    outputs.push(project.discover(DiscoveryContext::default()).await?);
    outputs.push(portless.discover(DiscoveryContext::default()).await?);
    let mut snap = if let Some(event_drops) = event_drops {
        SnapshotBuilder::from_adapter_outputs_with_config_and_event_drops(outputs, cfg, event_drops)
    } else {
        SnapshotBuilder::from_adapter_outputs_with_config(outputs, cfg)
    };
    let runs = lazyadmin_adapter_tracked::Registry::default()
        .list()
        .unwrap_or_default();
    for run in runs {
        if let Some(pid) = run.pid {
            for process in &mut snap.processes {
                if process.pid == pid as i32 {
                    process.lazyadmin_run_id = Some(RunId::new(run.id.clone()));
                }
            }
        }
    }
    Ok(snap)
}

pub async fn event_streams_for_config(
    cfg: &Config,
) -> Vec<futures::stream::BoxStream<'static, DiscoveryEvent>> {
    if !cfg.adapters.events.enabled {
        return Vec::new();
    }
    let mut streams = Vec::new();
    if cfg.adapters.sockets.enabled {
        let procfs = lazyadmin_adapter_procfs::ProcfsAdapter::new(cfg.clone());
        if let Some(stream) = procfs.watch().await {
            streams.push(stream);
        }
    }
    if cfg.adapters.container.enabled && cfg.adapters.container.events_enabled {
        let container = lazyadmin_adapter_container::ContainerAdapter::new();
        if let Some(stream) = container.watch().await {
            streams.push(stream);
        }
    }
    if cfg.adapters.systemd.enabled && cfg.adapters.systemd.events_enabled {
        let systemd = lazyadmin_adapter_systemd::SystemdAdapter::new(true);
        if let Some(stream) = systemd.watch().await {
            streams.push(stream);
        }
    }
    streams
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builds_snapshot_from_default_config() {
        let snap = build_snapshot(None).await.expect("snapshot builds");
        assert_eq!(snap.schema_version, "lazyadmin.snapshot.v1");
    }
}
