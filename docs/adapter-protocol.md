# Adapter protocol

Adapters return normalized managers, processes, listeners, workloads, projects, tracked runs, edges, warnings, health, and provenance into `lazyadmin-core`. v0.1 uses polling full snapshots; the trait shape leaves room for v0.2 `watch()` events. TUI and CLI consume core snapshots and action/log services only.
