# Adapter protocol

Adapters return normalized managers, processes, listeners, workloads, projects, tracked runs, edges, warnings, health, and provenance into `lazyadmin-core`. TUI and CLI consume core snapshots, discovery events, and action/log services only.

## v0.2 discovery additions

- `Listener.dual_stack_state` is additive and uses `not_applicable`, `confirmed_dual_stack`, `confirmed_v6_only`, `possible`, or `unknown`. Adapters must only emit `confirmed_*` when they have direct per-FD evidence such as `IPV6_V6ONLY`; otherwise IPv6 wildcard listeners stay `possible` and old warnings remain valid.
- Socket provenance may be `procfs` or opt-in `sock_diag`. The default path remains `/proc/net`.
- `DiscoveryAdapter::watch()` may return a stream of `lazyadmin.discovery_event.v1` events. Events are hints; snapshots remain the source of truth.
- Watch streams should emit `Heartbeat` periodically when no change occurs and `Degraded` when falling back or losing a live subscription.
- Portless discovery is polling-only and read-only. It emits additive `runtime: "portless"` workloads from route state, then core correlation links descendant procfs listeners with `workload_owns_listener` edges.
