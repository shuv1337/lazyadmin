# Discovery Event Schema v1

**Schema ID:** `lazyadmin.discovery_event.v1`

**Purpose:** Real-time discovery change events for TUI refresh and agent monitoring.

## Event Kinds

### Added
Emitted when a new entity is discovered.

```json
{
  "schema_version": "lazyadmin.discovery_event.v1",
  "kind": "added",
  "entity": {
    "kind": "listener",
    "id": "listener:..."
  },
  "timestamp": "2024-04-27T12:00:00Z"
}
```

### Removed
Emitted when an entity is no longer present.

```json
{
  "schema_version": "lazyadmin.discovery_event.v1",
  "kind": "removed",
  "entity": {
    "kind": "listener",
    "id": "listener:..."
  },
  "timestamp": "2024-04-27T12:00:00Z"
}
```

### Changed
Emitted when an entity's properties change.

```json
{
  "schema_version": "lazyadmin.discovery_event.v1",
  "kind": "changed",
  "entity": {
    "kind": "listener",
    "id": "listener:..."
  },
  "changes": [
    {
      "field": "state",
      "old": "running",
      "new": "stopped"
    }
  ],
  "timestamp": "2024-04-27T12:00:00Z"
}
```

### Heartbeat
Emitted periodically when no real changes occur, proving the source is alive.

```json
{
  "schema_version": "lazyadmin.discovery_event.v1",
  "kind": "heartbeat",
  "adapter": "procfs",
  "timestamp": "2024-04-27T12:00:00Z"
}
```

### Degraded
Emitted when an adapter enters a degraded state (e.g., sock_diag fallback).

```json
{
  "schema_version": "lazyadmin.discovery_event.v1",
  "kind": "degraded",
  "adapter": "procfs",
  "reason": "SOCK_DIAG_DOWNGRADED: permission denied, falling back to /proc/net",
  "timestamp": "2024-04-27T12:00:00Z"
}
```

## Entity References

All events reference entities by kind and ID:

```json
{
  "kind": "listener" | "process" | "workload" | "manager" | "project" | "run",
  "id": "string"
}
```

For processes, the ID is a composite key:

```json
{
  "kind": "process",
  "id": {
    "pid": 1234,
    "boot_id": "abc123",
    "start_time_ticks": 5678
  }
}
```

## Ordering Guarantees

- Events from a single adapter are ordered by timestamp
- Cross-adapter events may arrive out of order; consumers should sort by timestamp
- Heartbeat events are emitted at most once every 5 seconds per adapter
- Degraded events are emitted immediately when state changes

## Field Changes

The `Changed` event includes a list of field-level changes:

```json
{
  "field": "state",
  "old": "running",
  "new": "stopped"
}
```

Common fields:
- `state`: entity state (running, stopped, etc.)
- `exposure`: listener exposure (loopback, public, etc.)
- `dual_stack_state`: IPv6 dual-stack state
- `health`: workload health status
- `restart_policy`: restart policy changes

## Overflow Handling

When the event channel overflows:
- Oldest events are dropped
- `events_dropped` counter in `Snapshot.metadata` is incremented
- An `EVENTS_DROPPED` warning is emitted at the next snapshot

## Adapter-Specific Notes

### procfs
- Uses debounced polling (default 1s interval)
- Emits `Added`/`Removed`/`Changed` based on diff between scans
- Heartbeat every 5s when no changes

### container (Docker)
- Subscribes to Docker `/events` API
- Maps Docker events (`start`, `stop`, `die`, `restart`) to discovery events
- Reconnects with exponential backoff up to 30s
- Refreshes container state on event arrival

### systemd
- Subscribes to D-Bus `PropertiesChanged` signals
- Filters to units already in the graph
- Caps refetch concurrency to avoid dogpile
- Can be disabled via `adapters.systemd.events_enabled`
