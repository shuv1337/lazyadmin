# lazyadmin documentation

Welcome to the lazyadmin docs. lazyadmin is a Linux-first local runtime
control plane for developers and coding agents.

## Guides

- [Getting started](./getting-started.md) — Install, first commands, common workflows
- [CLI reference](./cli-reference.md) — Complete command reference with examples
- [Architecture](./architecture.md) — Workspace structure, data flow, and design

## Reference

- [TUI](./tui.md) — Ratatui interface, layouts, views, sorting
- [Keybindings](./keybindings.md) — Default bindings and overrides
- [Themes](./themes.md) — Built-in themes and palette customization
- [Action safety](./action-safety.md) — Danger levels, confirmation, PID reuse guards
- [Agent integration](./agent-integration.md) — Coding-agent skill usage

## Decision records

- [Adapter protocol](adapter-protocol.md)
- [Action safety design](action-safety.md)
- [Tracked run spawn](tracked-run-spawn-decision.md)
- [Pause/restart semantics](pause-restart-decision.md)
- [Portless adapter](portless-adapter.md)
- [TUI metrics panel](metrics-panel-decision.md)
- [Process tree view](process-tree-decision.md)
- [Discovery events](discovery-events-decision.md)
- [Sock-diag fallback](sock-diag-decision.md)
- [Container API](container-api-decision.md)

## JSON schemas

- [Snapshot v1](./schema/snapshot-v1.md)
- [Diff v1](./schema/diff-v1.md)
- [Doctor v1](./schema/doctor-v1.md)
