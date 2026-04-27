# Discovery events decision (PLAN-11)

## Decision

Discovery events use a new additive schema, `lazyadmin.discovery_event.v1`, and do not change the snapshot or doctor schema IDs. Snapshot and doctor remain v1 with optional additive fields.

## Event policy

- Adapters may expose `watch()` streams of normalized `DiscoveryEvent` values.
- Procfs uses debounced polling because `/proc/net` has no reliable subscription API.
- Container and systemd watch support is **not enabled** in v0.2. They remain poll-only adapters until Docker `/events` and systemd D-Bus subscriptions are implemented and tested.
- Consumers should treat events as hints and refresh snapshots for authoritative state.
- Events are ordered within a single adapter stream. Cross-adapter ordering is best effort.
- Overflow is reported by incrementing `Snapshot.metadata.events_dropped` and doctor `subsystems.events.dropped` when a bounded fan-in drops events.

## Deferred to PLAN-12/PLAN-13

- TUI consumption and view-level debouncing are PLAN-12 work.
- Native Docker `/events` reconnection and D-Bus `PropertiesChanged` fan-out are deferred; PLAN-12 must treat container/systemd watch streams as unavailable and refresh them through polling snapshots.
