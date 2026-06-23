use std::{path::PathBuf, time::Duration};

use futures::StreamExt;
use lazyadmin_core::{
    config::Config,
    correlate::EventFanIn,
    model::{DiscoveryEvent, Snapshot},
};
use tokio::sync::mpsc;

use crate::{build_snapshot_with_event_drops, event_streams_for_config};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSnapshotFeedSettings {
    pub snapshot_channel_capacity: usize,
    pub event_channel_capacity: usize,
    pub refresh_interval: Duration,
    pub event_debounce: Duration,
}

impl LiveSnapshotFeedSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            snapshot_channel_capacity: 4,
            event_channel_capacity: 64,
            refresh_interval: Duration::from_millis(cfg.ui.refresh.tick_ms),
            event_debounce: Duration::from_millis(cfg.ui.refresh.event_debounce_ms),
        }
    }

    pub fn with_refresh_interval(mut self, refresh_interval: Duration) -> Self {
        self.refresh_interval = refresh_interval;
        self
    }
}

pub struct LiveSnapshotFeed {
    pub snapshots: mpsc::Receiver<Snapshot>,
    pub events: mpsc::Receiver<DiscoveryEvent>,
}

pub fn spawn_live_snapshot_feed(
    cfg: Config,
    config_path: Option<PathBuf>,
    settings: LiveSnapshotFeedSettings,
) -> LiveSnapshotFeed {
    let (snapshot_tx, snapshots) = mpsc::channel(settings.snapshot_channel_capacity);
    let (event_tx, events) = mpsc::channel(settings.event_channel_capacity);
    tokio::spawn(async move {
        let streams = event_streams_for_config(&cfg).await;
        run_live_snapshot_feed(cfg, config_path, settings, streams, snapshot_tx, event_tx).await;
    });
    LiveSnapshotFeed { snapshots, events }
}

async fn run_live_snapshot_feed(
    cfg: Config,
    config_path: Option<PathBuf>,
    settings: LiveSnapshotFeedSettings,
    streams: Vec<futures::stream::BoxStream<'static, DiscoveryEvent>>,
    snapshot_tx: mpsc::Sender<Snapshot>,
    event_tx: mpsc::Sender<DiscoveryEvent>,
) {
    let has_events = !streams.is_empty();
    let (mut events, drops) = EventFanIn::new(
        streams,
        cfg.adapters.events.channel_capacity,
        settings.event_debounce,
    );
    let mut interval = tokio::time::interval(settings.refresh_interval);
    if !has_events {
        loop {
            interval.tick().await;
            match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                Ok(snapshot) => {
                    if snapshot_tx.send(snapshot).await.is_err() {
                        break;
                    }
                }
                Err(err) => tracing::debug!(error = %err, "snapshot refresh failed"),
            }
        }
        return;
    }
    loop {
        tokio::select! {
            _ = interval.tick() => {
                match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                    Ok(snapshot) => {
                        if snapshot_tx.send(snapshot).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => tracing::debug!(error = %err, "snapshot refresh failed"),
                }
            }
            event = events.next() => {
                match event {
                    Some(event) => {
                        let _ = event_tx.send(event).await;
                        tokio::time::sleep(settings.event_debounce).await;
                        match build_snapshot_with_event_drops(config_path.as_deref(), Some(&drops)).await {
                            Ok(snapshot) => {
                                if snapshot_tx.send(snapshot).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => tracing::debug!(error = %err, "event snapshot refresh failed"),
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use tokio::time::timeout;

    fn spawn_feed_with_streams(
        cfg: Config,
        settings: LiveSnapshotFeedSettings,
        streams: Vec<futures::stream::BoxStream<'static, DiscoveryEvent>>,
    ) -> LiveSnapshotFeed {
        let (snapshot_tx, snapshots) = mpsc::channel(settings.snapshot_channel_capacity);
        let (event_tx, events) = mpsc::channel(settings.event_channel_capacity);
        tokio::spawn(run_live_snapshot_feed(
            cfg,
            None,
            settings,
            streams,
            snapshot_tx,
            event_tx,
        ));
        LiveSnapshotFeed { snapshots, events }
    }

    #[test]
    fn settings_follow_config_refresh_values() {
        let mut cfg = Config::default();
        cfg.ui.refresh.tick_ms = 750;
        cfg.ui.refresh.event_debounce_ms = 25;
        let settings = LiveSnapshotFeedSettings::from_config(&cfg);

        assert_eq!(settings.refresh_interval, Duration::from_millis(750));
        assert_eq!(settings.event_debounce, Duration::from_millis(25));
        assert_eq!(settings.snapshot_channel_capacity, 4);
        assert_eq!(settings.event_channel_capacity, 64);
    }

    #[tokio::test]
    async fn polling_feed_emits_authoritative_snapshot_without_events() {
        let mut cfg = Config::default();
        cfg.adapters.events.enabled = false;
        cfg.ui.refresh.tick_ms = 50;
        let settings = LiveSnapshotFeedSettings::from_config(&cfg);
        let mut feed = spawn_live_snapshot_feed(cfg, None, settings);

        let snapshot = timeout(Duration::from_secs(5), feed.snapshots.recv())
            .await
            .expect("snapshot received before timeout")
            .expect("snapshot channel open");

        assert_eq!(snapshot.schema_version, "lazyadmin.snapshot.v1");
        assert!(matches!(
            feed.events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn event_hint_triggers_authoritative_snapshot_before_poll_tick() {
        let mut cfg = Config::default();
        cfg.ui.refresh.tick_ms = 60_000;
        cfg.ui.refresh.event_debounce_ms = 0;
        let settings = LiveSnapshotFeedSettings::from_config(&cfg);
        let streams = vec![stream::iter([DiscoveryEvent::heartbeat("procfs")]).boxed()];
        let mut feed = spawn_feed_with_streams(cfg, settings, streams);

        let event = timeout(Duration::from_secs(5), feed.events.recv())
            .await
            .expect("event received before poll tick")
            .expect("event channel open");
        assert_eq!(event.adapter.as_deref(), Some("procfs"));

        let snapshot = timeout(Duration::from_secs(5), feed.snapshots.recv())
            .await
            .expect("event-triggered snapshot received")
            .expect("snapshot channel open");
        assert_eq!(snapshot.schema_version, "lazyadmin.snapshot.v1");
    }

    #[tokio::test]
    async fn dropped_event_counts_propagate_to_snapshots() {
        let mut cfg = Config::default();
        cfg.adapters.events.channel_capacity = 1;
        cfg.ui.refresh.tick_ms = 60_000;
        cfg.ui.refresh.event_debounce_ms = 0;
        let settings = LiveSnapshotFeedSettings::from_config(&cfg);
        let streams = vec![
            stream::iter([
                DiscoveryEvent::heartbeat("a"),
                DiscoveryEvent::heartbeat("b"),
                DiscoveryEvent::heartbeat("c"),
            ])
            .boxed(),
        ];
        let mut feed = spawn_feed_with_streams(cfg, settings, streams);

        let snapshot = timeout(Duration::from_secs(5), feed.snapshots.recv())
            .await
            .expect("event-triggered snapshot received")
            .expect("snapshot channel open");

        let dropped = snapshot
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.events_dropped)
            .unwrap_or_default();
        assert!(dropped > 0, "bounded fan-in drops should be recorded");
        assert!(
            snapshot
                .warnings
                .iter()
                .any(|warning| warning.code == "EVENTS_DROPPED")
        );
    }
}
