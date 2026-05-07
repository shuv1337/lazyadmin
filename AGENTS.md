# lazyadmin Project Notes

## Current state

This repository now contains the v0.4 Rust workspace for `lazyadmin` after PLAN-01 through PLAN-14:

- `Cargo.toml` — workspace root (`resolver = "2"`) with shared dependencies.
- `crates/lazyadmin-core` — core models, graph/discovery contracts, config loader, redaction, selectors, snapshot/diff JSON contract, and telemetry primitives.
- `crates/lazyadmin-cli` — Clap command skeleton with `export`, `diff`, `config check`, read-only views, `doctor`, `logs`, `free`, and pause-registry commands. Runtime mutation is conservative and direct-process free validates `ProcessKey` before signaling.
- `crates/lazyadmin-adapter-portless` — read-only portless route discovery for `PORTLESS_STATE_DIR`, `~/.portless`, and existing `/tmp/portless` legacy state. It emits `RuntimeKind::Portless` managers/workloads and never mutates portless state.
- `crates/lazyadmin-tui` contains the Ratatui interface: responsive rendering of core view-models, Process Tree and Metrics views, theme/keybinding support, terminal panic guard, and live-refresh coalescing. View/render tests cover 120/90/70/50 column modes; avoid launching the TUI interactively in automation and prefer `lazyadmin tui --headless --json`.
- `lazyadmin-spec-v0_2.md` — source specification for the Linux-first Rust + Ratatui local runtime control plane.
- `docs/spec.md` — symlink to the source spec; do not fork divergent specs silently.
- `docs/schema/` and `testdata/snapshots/` — initial public JSON contract docs and fixtures.
- `PLAN-*.md` — implementation history/checklists derived from the spec and assumption review. PLAN-05 covers the TUI, agent skill, docs, CI, packaging, and v0.1 acceptance record; PLAN-12 covers the v0.2 TUI polish; PLAN-13 covers the v0.3 portless adapter and manager-aware `free` release; PLAN-14 covers the v0.4 read-only Web UI observation layer (`lazyadmin-runtime` + `lazyadmin-web`).
- `skills/lazyadmin-agent/` — shipped coding-agent skill with always-do rules, cheatsheet, schema notes, examples, and install script.
- `scripts/build-skill-tarball.sh` — builds the release skill artifact `lazyadmin-agent-skill-v<version>.tar.gz`.

## Intended architecture

`lazyadmin` is planned as a Linux-first Rust workspace with:

- `crates/lazyadmin-core` for normalized graph models, correlation, actions, config, redaction, JSON schemas, diffs, and telemetry primitives.
- `crates/lazyadmin-cli` for CLI parsing and human/JSON output, including `export`, `diff`, `config check`, tracked-run commands, and read-only `ps`/`public`/`conflicts`/`projects` views.
- `crates/lazyadmin-tui` for the Ratatui interface.
- Adapter crates for procfs/sockets, systemd, containers, projects, and tracked runs.
- `skills/lazyadmin-agent` for AI-agent integration guidance.

## Important validated assumptions and caveats

- The spec's `systemd-run --user --scope` path needs an implementation spike before relying on detached-run and journal-log behavior. The plan keeps scopes as the v0.1 default candidate, but requires proving or downgrading claims.
- Bollard is confirmed as a Docker Engine API Rust client, but the plan does not assume automatic Podman socket discovery without a spike. Probe Docker and Podman sockets explicitly.
- `GetUnitByPIDFD`, `GetUnitByPID`, `GetUnitByControlGroup`, `StopUnit`, `RestartUnit`, `KillUnit`, `StartTransientUnit`, and unit-listing methods exist in systemd's D-Bus Manager interface, but runtime availability varies by systemd version.
- Exact IPv6 dual-stack (`IPV6_V6ONLY`) detection is not proven from `/proc/net` alone. v0.1 should label it best-effort unless the adapter can prove the per-socket option.
- PLAN-11 keeps sock_diag opt-in (`adapters.sockets.preferred = "proc"` by default). The v0.2 implementation has a feature-gated spike-safe sock_diag path for fallback/parity/provenance plumbing; native netlink enumeration remains deferred until live parity is proven.
- Discovery events use `lazyadmin.discovery_event.v1`. In v0.2, procfs watch is debounced polling through the bounded fan-in; Docker-compatible container `/events` and systemd D-Bus signals are native refresh hints. Snapshot polling remains authoritative for all adapter state.
- PLAN-12 TUI live refresh treats DiscoveryEvent messages as hints only; snapshot polling remains the authoritative state source, especially for container and systemd data.
- TUI config knobs live under `[ui.theme]`, `[ui.keybindings]`, and `[ui.refresh]` (`tick_ms`, `event_debounce_ms`, `max_redraw_hz`). `lazyadmin config check --json` includes resolved keybindings for automation.
- Built-in themes are Night Owl-aligned: `default-dark` and its alias `night-owl` ship Sarah Drasner's canonical Night Owl palette (`#011627` bg, `#d6deeb` fg, `#ecc48d` accent, `#82aaff` info, `#addb67` ok, `#f78c6c` warning, `#c792ea` degraded, `#ef5350` error, `#1d3b53` selection, `#637777` footer); `default-light` / `night-owl-light` ship Night Owl Light. `high-contrast` and `solarized-dark` are unchanged, and `colorblind-safe` keeps Night Owl surfaces while swapping risk/marker/status slots. When tweaking themes, edit `Theme::builtin` in `crates/lazyadmin-tui/src/lib.rs` and keep `palette_entries` + `tests::theme_builtins_validate_and_downgrade` in sync.
- Event overflow counts are reusable via the core `EventDropCounter`; long-lived runtimes should pass that counter into snapshot/doctor builders. Stateless CLI `doctor`/`export` runs cannot observe historical fan-in drops and report that limitation explicitly.
- `pause-restart` semantics are recorded in `docs/pause-restart-decision.md`: prefer systemd runtime `Restart=no` overrides, use Docker update API for verified container executors, defer Podman mutations to a later release, and keep lazyadmin-owned pause records in `$XDG_STATE_HOME/lazyadmin/pauses`.
- Portless interop is read-only except for `free`: route state is read from `routes.json`, aliases use `pid = 0`, orphan cleanup is only a `doctor` hint to run `portless prune`, and `lazyadmin free <port>` sends `SIGTERM` to the portless CLI `ProcessKey` rather than the descendant dev-server process.
- The read-only Web UI is split across `crates/lazyadmin-runtime` (shared snapshot/event assembly) and `crates/lazyadmin-web` (loopback-only Axum server plus embedded static app). `lazyadmin web` refuses non-loopback binds in v1 and exposes only read-only API routes; use `--port 0 --no-open` for smoke tests.
- The Web UI static app is now three files — `crates/lazyadmin-web/static/index.html`, `app.css`, `app.js` — each embedded via `include_str!` and served with `Cache-Control: no-store` in dev. There is no bundler, no framework, and no inline JS. Read-only API surface: `/api/health`, `/api/snapshot`, `/api/digest`, `/api/doctor`, `/api/header_pip`, `/api/inspector?kind=&id=`, `/api/rail`, `/api/views/overview`, `/api/entities/:kind/:id`, `/api/events`. The Web UI nav is the same `RAIL_ENTRIES` constant the TUI rail consumes, grouped client-side into Triage / Inventory / Diagnostics.
- PLAN-15 #20 footer/header chrome is landed in the TUI: the footer is static contextual key hints padded through `pad_to_width`, transient statuses route through the toast queue and render above the footer, confirmation hints live inside the modal, and the header consumes `HeaderPip` for health/freshness/drop slots. Do not put volatile status copy back into the footer.
- PLAN-15 #21 metrics polish is landed: TUI/Web Metrics copy must keep the stateless drop-counter limitation explicit, idle adapter event rate is an affirmative empty state, listener histograms use full-word labels (`Listeners`, `Public`, `Conflicts`, `Orphans`), and metric captions come from `lazyadmin_core::doctor::metric_caption`.
- `lazyadmin_runtime::view_model::inspector::InspectorView` is the typed per-entity-kind inspector shape (Listener / Workload / Process / Project / Manager / TrackedRun / WarningGroup). It exposes `lookup(snapshot, kind, id)` for the Web `/api/inspector` route and `to_sections()` to flatten any variant into a renderer-agnostic `Vec<InspectorSection>` keyed by stable headings (`IDENTITY`, `PROCESS`, `RELATED`, `PROJECT`, `CONFIDENCE`, `ACTIONS`, `WARNINGS`, etc.). Listener IDs and other identifiers are full-fidelity — do not truncate them in renderer code; section rows that name an entity carry a `jump_target` so renderers can attach one-key navigation. The Web UI mirrors `to_sections()` in JS, and the TUI now projects `InspectorView` into `InspectorSectionVm` before rendering/wrapping section rows, including `[1]…[9]` related-entity jumps, `[v] view all related` listener overflow, and action-key confirmation modals with command previews; keep future inspector fields in the runtime view-model rather than hand-formatting rows in `crates/lazyadmin-tui/src/lib.rs`.
- Confidence signals classify the existing `Provenance.adapter` string into a fixed enum (`ConfidenceSignal::{ProcfsPidInode, ContainerInspect, CgroupCorrelation, ManagerAttribution, TrackedRunRegistry, PortlessRoutes, BestEffort}`). Unrecognized adapters fall through to `BestEffort` so the user sees the truthful label rather than a confident wrong one. No `Provenance` schema change — `lazyadmin export --json` and `doctor --json` stay byte-stable.
- The UX-overhaul shared projection layer starts in `crates/lazyadmin-runtime/src/view_model/` (`digest`, `doctor_groups`, `header_pip`, `inspector`). Keep new TUI/Web grouping and summary logic there instead of duplicating snapshot projections in each UI.
- PLAN-15a digest landing is implemented: `ViewKind::Overview` is the default TUI view, `lazyadmin overview --json` emits the shared `Digest`, and the Web UI exposes `/api/digest` with the default route rendering the digest.
- PLAN-15 #19 rail collapse is partially landed: the canonical rail order lives in `lazyadmin_runtime::view_model::RAIL_ENTRIES` (`Overview`, `Listeners`, `Workloads`, `Processes`, `Doctor`, `Metrics`), and the TUI consumes it. Legacy filtered views such as `Public`, `Conflicts`, and `Orphans` remain programmatically addressable but are hidden from the rail; continue wiring Web nav to the same constant in PLAN-15c.
- PLAN-15 #18 visual hierarchy is landed except for refreshed dogfood screenshots: TUI listener rows carry a signal slot (`●` public, `◐` LAN, blank loopback) plus an independent marker slot (`┃` conflict, `▎` tracked/project); Process Tree, Workloads, summary tables, and digest sections use the same marker vocabulary; header/chip exposure counts use semantic risk colors; system rows dim with `system_noise`; and the Web listener glyph mirrors the LAN/public distinction. `PaletteMode::Monochrome` exists so tests can assert glyph-only distinguishability, and `colorblind-safe` keeps Night Owl surfaces while swapping risk/marker/status slots.
- Warning actionability metadata lives in `lazyadmin_core::doctor::{WARNING_CODE_REGISTRY, classify}`. Add every newly emitted `Warning.code` there with an explicit `WarningTier`, label, and remediation; do not add tier/remediation fields to snapshot JSON.

## Development standards

- Telemetry is day-zero: every adapter scan, correlation pass, action plan/execution, CLI command, TUI refresh, and log stream should emit structured `tracing` spans/events with stable IDs where possible.
- Prefer safe, reversible actions. All mutating runtime actions need dry-run output, explicit confirmation policy, structured result reporting, and post-action verification.
- JSON output is a public contract for scripts and agents. Add schema/golden tests before changing it.
- Keep human output and TUI view models as projections of the core graph; do not duplicate ownership/correlation logic in UI crates.

## Validation commands

Run these before handing off foundation changes:

```bash
cargo metadata --format-version=1
cargo test -p lazyadmin-adapter-portless
cargo test -p lazyadmin-cli --features integration-portless free_portless_app
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p lazyadmin-cli -- --help
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- ps --json
cargo run -p lazyadmin-cli -- public --json
cargo run -p lazyadmin-cli -- conflicts --json
cargo run -p lazyadmin-cli -- projects --json
cargo run -p lazyadmin-cli -- overview --json
cargo run -p lazyadmin-cli -- diff testdata/snapshots/empty.json testdata/snapshots/empty.json --json
cargo run -p lazyadmin-cli -- config check --json
cargo run -p lazyadmin-cli -- doctor --json
cargo run -p lazyadmin-cli -- events --once --json
cargo run -p lazyadmin-cli -- tui --headless --json
cargo test -p lazyadmin-runtime -p lazyadmin-web
cargo run -p lazyadmin-cli -- web --port 0 --no-open
cargo test -p lazyadmin-tui render_views
cargo test -p lazyadmin-tui live_refresh
cargo test -p lazyadmin-tui process_tree
cargo test -p lazyadmin-tui metrics
cargo test -p lazyadmin-tui theme
cargo test -p lazyadmin-tui keybindings
```

Later plans may add Linux integration tests such as:

```bash
cargo test --workspace --features integration-linux -- --ignored
```

Update this file when the workspace layout, commands, or operational caveats change.
