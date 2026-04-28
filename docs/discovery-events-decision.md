# Discovery events decision (PLAN-11)

## Decision

Discovery events use a new additive schema, `lazyadmin.discovery_event.v1`, and do not change the snapshot or doctor schema IDs. Snapshot and doctor remain v1 with optional additive fields.

## Event policy

- Adapters may expose `watch()` streams of normalized `DiscoveryEvent` values.
- Procfs uses debounced polling because `/proc/net` has no reliable subscription API.
- Container and systemd watch support is enabled as native activity-hint streams: Docker-compatible `/events` for container runtimes and systemd D-Bus signals for systemd.
- Consumers should treat events as hints and refresh snapshots for authoritative state.
- Events are ordered within a single adapter stream. Cross-adapter ordering is best effort.
- Overflow is tracked by the shared bounded fan-in `EventDropCounter`. Long-lived consumers should pass that counter into snapshot/doctor builders so `Snapshot.metadata.events_dropped`, `EVENTS_DROPPED` warnings, and doctor `subsystems.events.dropped` all come from the same source. The stateless CLI `doctor` command has no long-lived fan-in to observe, so it reports `drop_counter_observable=false` and `drop_counter_source="unavailable_in_stateless_cli_doctor"` rather than inventing a count.

## Config

Global events are controlled by `[adapters.events] enabled = true`. Container and systemd event streams can be independently disabled with `adapters.container.events_enabled = false` and `adapters.systemd.events_enabled = false`.

## Current Limits

- Docker `/events` and D-Bus `PropertiesChanged`/job signals are treated as refresh hints. They do not replace snapshot polling, and they do not currently carry complete refetched field-level state by themselves.
