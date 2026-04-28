# PLAN-12 — v0.2 TUI Polish: Real Rendering, Process Tree, Metrics, Themes, Keybindings, Live Refresh

Source: `lazyadmin-spec-v0_2.md` §12, §21.2, §24, §27.
Depends on: `PLAN-10-v0_2-index-and-assumption-review.md`, `PLAN-11-v0_2-discovery-upgrades.md`.
Goal: turn the v0.1 TUI skeleton (view-models + dispatcher + panic guard) into a fully rendered, themable, and customizable Ratatui application; add Process Tree and Metrics views; wire it to the new `DiscoveryEvent` streams from PLAN-11.

## Implementation principles

- The TUI is a projection of core. No new domain logic in the TUI crate — it consumes view-models and calls the same action planner/executor used by the CLI.
- Backward-compatible defaults: a user who runs `lazyadmin` after upgrade should see the v0.1 view-models rendered, with no config changes required.
- Configuration is opt-in: themes and keybinding overrides live in TOML files that are validated at load time.
- Performance: keystroke-to-redraw under 50ms; live event-driven redraws coalesced to at most 30Hz.
- All terminal state changes go through the existing panic/restore guard.

## Cross-cutting tasks (do early)

- [x] Add config sections:
  - [x] `[ui.theme]` — `name` (built-in name) or `path` (TOML file path).
  - [x] `[ui.keybindings]` — `path` (TOML file path) or inline `[ui.keybindings.overrides]` (table mapping action → key spec).
  - [x] `[ui.refresh]` — `tick_ms` (default 500), `event_debounce_ms` (default 100), `max_redraw_hz` (default 30).
- [x] Add `docs/themes.md` and `docs/keybindings.md` describing format and validation.
- [x] Add a `lazyadmin tui` subcommand alias (already implicit when `lazyadmin` runs with no args). Add explicit `lazyadmin tui --headless` to dump the current default view-model as JSON for debugging without a real terminal.
- [x] Telemetry spans: `tui.start`, `tui.stop`, `tui.render`, `tui.input`, `tui.theme.load`, `tui.theme.apply`, `tui.keybind.load`, `tui.keybind.conflict`, `tui.event.received`, `tui.refresh.coalesced`.

Validation:

```bash
cargo run -p lazyadmin-cli -- tui --headless --json | jq '.layout.width, .panes | length'
```

## Phase 1 — Real rendering of v0.1 view-models

- [x] Implement `render(view_model: &ViewModel, frame: &mut Frame, area: Rect, theme: &Theme)` for each existing view:
  - [x] Everything view (default landing pane) using `Table` with badge column.
  - [x] Ports view.
  - [x] Public view.
  - [x] Conflicts view.
  - [x] Projects view.
  - [x] Managers view.
  - [x] Orphans view.
  - [x] Tracked Runs view.
  - [x] Logs view (uses scrollable `Paragraph` + `Wrap { trim: false }`).
  - [x] Doctor view (severity badges + grouped sections).
- [x] Implement responsive layout switches (≥100 / 80–99 / 60–79 / <60 cols) using `ratatui::layout::Layout`. The <60 case shows a friendly CLI-hint banner and no panes.
- [x] Inspector pane:
  - [x] Render identity, state, runtime, ports, process tree summary, project, tracked metadata, restart policy, logs preview (last N lines), warnings, actions (planner output), provenance list with collapse/expand.
  - [x] Use a clear "selected entity" highlight in the active list pane.
- [x] Two-tier filter is rendered as a footer status (`hidden: 12 system services. press S to toggle`).
- [x] Snapshot/golden tests for rendered output via `ratatui::backend::TestBackend` capturing buffers at widths 120/90/70/50.

Tests:

- [x] golden test per view at each width.
- [x] panic guard restores terminal in unit test (`std::panic::catch_unwind`).
- [x] hidden-count footer appears when filter active.

Validation:

```bash
cargo test -p lazyadmin-tui render_views
```

## Phase 2 — Live refresh wired to `DiscoveryEvent`

- [x] Replace v0.1 placeholder snapshot polling with:
  - [x] One `tokio::task` that subscribes to `lazyadmin-core` event channel from PLAN-11 Phase 4.
  - [x] Tick task (`tick_ms`) for periodic full-snapshot refresh as a safety net.
  - [x] A small per-view debounce keyed by `event_debounce_ms` to avoid render storms.
  - [x] Hard cap on redraws via `max_redraw_hz`.
- [x] When `events_dropped > 0`, render a banner `EVENTS DROPPED — refresh may lag`.
- [x] On adapter `Degraded` event, render a status pill in the footer with adapter name and reason.
- [x] Input task remains independent; never blocks on snapshot.

Tests:

- [x] simulated event stream causes view to refresh within debounce window.
- [x] simulated burst exceeds redraw cap and gets coalesced.
- [x] dropped-event banner appears and clears when counter resets.

Validation:

```bash
cargo test -p lazyadmin-tui live_refresh
```

## Phase 3 — Process Tree view

- [x] Build a new `ViewModel::ProcessTree` derived from `Snapshot.processes` + `Edge::ProcessParent`:
  - [x] Root by manager / project where applicable; fall back to PID 1 / session leader.
  - [x] Each node carries `ProcessKey`, runtime kind, owning workload (if any), warnings.
- [x] Decide tree widget approach:
  - [x] Option A: `tui-tree-widget` crate. Document license/maintenance and pin a version.
  - [x] Option B: render manually using `Table` + `└──` ASCII guides; portable, no extra dep.
  - [x] Decision recorded in `docs/process-tree-decision.md`.
- [ ] Keymap entry: `t` opens the Process Tree view scoped to the currently selected entity (or whole graph if no selection). `t` again on a node toggles expand/collapse.
- [ ] Inspector integration: when a process node is selected, the existing inspector pane shows its details (ports owned, project, tracked metadata).
- [x] Search (`/`) operates on visible tree rows.
- [x] Stable selection across refreshes uses `ProcessKey`.

Tests:

- [x] tree built from fixture snapshot has the expected shape and ordering.
- [ ] expanding/collapsing nodes does not lose selection.
- [x] PID reuse during refresh is handled: a node with the same PID but different start time becomes a new node.

Validation:

```bash
cargo test -p lazyadmin-tui process_tree
```

## Phase 4 — Metrics panel

- [x] Build a new `ViewModel::Metrics` populated from existing snapshot data:
  - [x] Counts: listeners (by exposure), workloads (by runtime), warnings (by severity), tracked runs.
  - [x] Rates: derive simple deltas from previous snapshot (no new `/proc` polling); store last N snapshots in core if necessary.
  - [ ] Adapter health: latency / event throughput / drops per adapter.
- [x] Render with `ratatui::widgets::{Sparkline, Gauge, Chart}`:
  - [ ] Sparkline for per-adapter event rate.
  - [x] Gauge for `events_dropped` saturation against capacity.
  - [x] Bar chart for warnings by severity.
- [x] Keymap entry: `m` opens Metrics view.
- [x] Decision: rates use diff between consecutive snapshots, **not** EWMA, to keep core change minimal. Record in `docs/metrics-panel-decision.md`.
- [x] Document limitations: numbers are coarse, intended for situational awareness, not monitoring.

Tests:

- [x] metrics view-model derives expected counts from fixture.
- [x] rate calculation across two fixtures produces non-negative numbers.
- [ ] adapter event sparkline reads from telemetry counters via a thin in-memory ring buffer.

Validation:

```bash
cargo test -p lazyadmin-tui metrics
```

## Phase 5 — Configurable keybindings

- [x] Define a TOML schema:

  ```toml
  [keybindings]
  inherit = "default" # or "vim", "readline" once we add presets

  [keybindings.overrides]
  quit          = "Q"
  free_port     = "ctrl+f"
  open          = "o"
  copy_diag     = "y"
  ```

- [x] Implement parser and validator in `lazyadmin-core/src/config/keybindings.rs`:
  - [x] Map every action enum to one or more bindings.
  - [x] Reject duplicate bindings with a clear error including line/column when possible.
  - [x] Reject unknown action names with a suggestion (Levenshtein closest match).
- [x] Expose action enum publicly: `KeybindAction { Quit, Help, NextPane, PrevPane, OpenPalette, ToggleFilter, ... }`. Cover every key from PLAN-05 Phase 6.
- [x] Apply at TUI start; emit `tui.keybind.load` and `tui.keybind.conflict` telemetry events.
- [x] Help overlay (`?`) reads from the resolved keybinding map, not a hardcoded table, so customization shows.
- [x] Add `lazyadmin config check` extension: validate keybinding file when present.

Tests:

- [x] default config produces v0.1 keymap exactly.
- [x] override changes only the overridden actions.
- [x] duplicate binding rejected with helpful error.
- [x] unknown action rejected with suggestion.
- [x] help overlay reflects overrides.

Validation:

```bash
cargo run -p lazyadmin-cli -- config check --json | jq '.keybindings'
cargo test -p lazyadmin-core keybindings
```

## Phase 6 — Themes

- [x] Define a `Theme` struct covering all colored surfaces:
  - [x] base bg/fg, accents, severity colors (info/warning/degraded/error/ok), badge palette (PUBLIC, CONFLICT, ROOT, etc.), inspector divider, selection, footer.
- [x] Built-in themes:
  - [x] `default-dark`
  - [x] `default-light`
  - [x] `high-contrast`
  - [x] `solarized-dark`
- [ ] User themes:
  - [ ] TOML files under `$XDG_CONFIG_HOME/lazyadmin/themes/<name>.toml`.
  - [ ] Loaded by name or absolute path.
- [x] Validation:
  - [ ] every required surface key present, otherwise inherit from `default-dark`.
  - [x] color values accepted as `#RRGGBB`, `#RRGGBBAA` (alpha ignored), or named ANSI (`red`, `bright-blue`).
  - [x] Each theme declares a `fallback_palette = "16" | "256" | "truecolor"` so we can downgrade when terminal capability is limited.
- [x] Integration with crossterm:
  - [ ] Detect color support via `crossterm::style::available_color_count()` (or `COLORTERM` env).
  - [ ] Downgrade theme to declared fallback when needed; render a one-time hint to the footer ("limited color terminal — using <fallback>").

Tests:

- [x] each built-in theme parses and round-trips.
- [ ] missing keys fall back to default-dark.
- [x] truecolor theme loaded on a 16-color test backend uses the fallback palette.
- [x] invalid color string yields a precise error.

Validation:

```bash
cargo test -p lazyadmin-tui themes
cargo run -p lazyadmin-cli -- tui --headless --theme high-contrast --json | jq '.theme.name'
```

## Phase 7 — Help, palette, copy-diagnostic, open

- [x] Help overlay (`?`):
  - [x] modal scrollable list of action → key mappings, sections grouped (Navigation, Views, Actions, Misc).
  - [x] reflects active keybindings.
- [x] Command palette (`:`):
  - [x] keep v0.1 entries; expand to include Process Tree (`:process-tree`), Metrics (`:metrics`), Theme switch (`:theme <name>`), Reload config (`:reload`).
  - [ ] reload command re-reads config and applies new theme/keybindings without restarting.
- [x] Copy diagnostic (`y`):
  - [ ] uses `arboard`; falls back to `wl-copy`/`xclip` shellout; on failure shows a footer message and writes the same diagnostic to `$XDG_STATE_HOME/lazyadmin/copies/<timestamp>.md`.
- [ ] Open URL (`o`):
  - [x] only enabled for localhost TCP listeners on common HTTP ports unless user opts in via config (`actions.open_non_loopback = false` default).
  - [ ] uses `open` crate / `xdg-open` shellout with explicit URL argument escaping.

Tests:

- [x] help overlay renders all bindings.
- [x] palette command `:reload` triggers config re-read (mocked).
- [ ] copy-diagnostic falls back to file on simulated clipboard failure.
- [x] open refuses non-loopback by default.

Validation:

```bash
cargo test -p lazyadmin-tui help_palette_open_copy
```

## Phase 8 — Documentation and acceptance

- [x] Add docs:
  - [x] `docs/tui.md` — full TUI guide for users (panes, keys, palette, themes, troubleshooting).
  - [x] `docs/themes.md` — built-ins, file format, color rules, fallbacks.
  - [x] `docs/keybindings.md` — action list, override examples, validation errors.
  - [x] `docs/process-tree-decision.md` — chosen widget approach.
  - [x] `docs/metrics-panel-decision.md` — rate calculation approach.
  - [x] `docs/discovery-events-decision.md` — created in PLAN-11; link from here.
- [x] Update `README.md` with screenshots/asciicasts (optional but encouraged) and a one-liner about themes/keybindings.
- [x] Update agent skill:
  - [x] note that the TUI now renders fully and supports themes/keybindings,
  - [x] no behavioral change for agents — JSON contracts are still the recommended interface.
- [x] Write `docs/acceptance-v0_2.md` mirroring the v0.1 file:
  - [x] sock_diag opt-in works,
  - [x] dual-stack proof present and honest,
  - [x] `lazyadmin events --json` streams,
  - [x] all v0.1 view-models render,
- [ ] Process Tree and Metrics views work,
  - [x] keybinding overrides accepted,
  - [x] themes load and downgrade safely,
- [ ] copy-diagnostic works or falls back,
  - [x] no JSON regressions.
- [x] AGENTS.md update: state, validation commands, new config knobs.

Validation:

```bash
cargo doc --workspace --no-deps
cargo run -p lazyadmin-cli -- tui --headless --json
```

## Phase 9 — Release prep (within v0.2 sprint)

- [ ] Bump workspace version to `0.2.0` in `Cargo.toml` and crate `Cargo.toml` files.
- [x] Update `CHANGELOG.md` with the v0.2 changeset.
- [ ] Re-run the v0.1 release smoke (build skill tarball, `cargo install --path crates/lazyadmin-cli --locked`, then uninstall) to ensure packaging still works.
- [ ] Tag `v0.2.0` after final review (manual step; do not auto-tag from agents).
- [ ] CI: keep gating identical to v0.1; add new tests under existing fmt/clippy/unit/fixture/golden buckets.

## Done criteria

- [x] All v0.1 view-models render with the new theme system at the four supported widths.
- [ ] Process Tree and Metrics views ship and have golden tests.
- [x] Live refresh consumes events from PLAN-11 with bounded redraws and visible degraded indicators.
- [x] Keybinding overrides validated at config load with helpful errors.
- [x] Themes load, validate, and downgrade for limited-color terminals.
- [ ] Help overlay, palette, copy-diagnostic, and open all work with sane fallbacks.
- [x] Docs cover TUI, themes, keybindings, decision records.
- [x] `cargo fmt`/`clippy`/`test`/`doc` all pass.
- [x] `docs/acceptance-v0_2.md` records v0.2 acceptance with PASS/PARTIAL/DEFERRED entries.

## Handoff notes for v0.3

- Live container/systemd events streams already exist (PLAN-11) — v0.3 can layer richer per-event UI.
- Podman actions and packaging targets (Homebrew tap, Nix flake) remain in the v0.3 backlog.
- Configurable per-pane keybindings, more theme presets, and TUI export-to-image are nice-to-haves to discuss at v0.3 planning.

## Implementation notes

- Implemented PLAN-12 as a v0.2 TUI polish pass with explicit `lazyadmin tui --headless --json`, snapshot-derived rendering, Process Tree, Metrics, themes, keybinding validation, live refresh coalescing, docs, and acceptance notes.
- Follow-up fixed interactive runtime wiring: `lazyadmin tui` now receives resolved theme/keybindings/refresh settings, procfs `DiscoveryEvent`s trigger debounced snapshot refreshes, periodic polling remains authoritative, and keybinding overrides drive actual dispatch.
- Live DiscoveryEvent handling intentionally treats procfs events as redraw/refresh hints; snapshot polling remains authoritative. Container/systemd native event streams remain deferred from PLAN-11.
- Process Tree and Metrics are v0.2 situational views, not complete interaction surfaces: expand/collapse, selected-process inspector details, per-adapter latency/throughput, and telemetry-backed sparklines remain unchecked.
- Copy diagnostic has a deterministic file fallback helper; full interactive clipboard/open shell integration is deferred. Palette reload/theme entries are present as non-interactive/testable surfaces, not a complete runtime config reloader.
- Review follow-up fixed three release-readiness issues before final PLAN-12 review: `diff <before> -` now compares against the current snapshot, `free --dry-run --json` emits JSON, and partial config TOML files merge with defaults as documented. Free-port docs now accurately describe the current direct-process executor and mark manager-aware stop plans as deferred.
- Workspace version was not bumped to 0.2.0 and no tag was created because release/tagging still needs user review.
