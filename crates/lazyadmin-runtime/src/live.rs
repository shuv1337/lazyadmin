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
    });
    LiveSnapshotFeed { snapshots, events }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

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
}
