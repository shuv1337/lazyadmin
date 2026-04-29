# Changelog

## 0.4.0

- PLAN-14: read-only Web UI observation layer. New `lazyadmin-runtime` crate extracts shared snapshot/event assembly, and the new `lazyadmin-web` crate ships a loopback-only Axum server with an embedded static UI shell.
- New `lazyadmin web` CLI command: defaults to `127.0.0.1:7749` (or `--port 0` for ephemeral), refuses non-loopback binds, and exposes only read-only `/api/health`, `/api/snapshot`, `/api/doctor`, and `/api/events` routes.
- Snapshot and doctor data remain projections of `lazyadmin.snapshot.v1`; the web UI does not introduce any mutating runtime actions.

## 0.3.0

- Adds a read-only portless adapter, manager-aware `free` dispatch that sends `SIGTERM` to the portless CLI for portless-owned ports, and `adapter:portless` doctor checks.
- New additive enum variants: `RuntimeKind::Portless` (`"portless"`) and `ActionKind::PortlessStop` (`"portless_stop"`). Strict JSON-schema consumers should regenerate or tolerate these variants.
- Verified against a real `portless 0.11.1` install: `lazyadmin ps` labels portless-owned listeners, `lazyadmin doctor` reports `adapter:portless`, `lazyadmin free <port>` removes the route, and orphaned routes are reported with a `portless prune` hint.

## 0.2.0

- PLAN-12 TUI polish: interactive `lazyadmin tui` receives resolved refresh/theme/keybinding runtime settings, uses procfs discovery events as refresh hints with snapshot polling as the authoritative fallback, and applies keybinding overrides to dispatch.
- Process Tree supports selected-process inspector details, stable `ProcessKey` selection, search, and expand/collapse from the `t` key path.
- Metrics includes snapshot-derived counts/rates plus an in-memory adapter event ring for throughput, drop counts, and sparklines.
- Themes load from built-ins, explicit paths, or `$XDG_CONFIG_HOME/lazyadmin/themes/<name>.toml`; partial theme files inherit from `default-dark` and downgrade for limited-color terminals.
- Palette reload/theme entries, guarded local URL opening, and copy-diagnostic clipboard/file fallback are implemented.

## 0.1.0

Initial Linux-first release: CLI, core discovery/action safety, Ratatui MVP, agent skill, docs, CI, and packaging workflows.
