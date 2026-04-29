# PLAN-14: Read-only Web UI Observation Layer

## Goal

Add a local, read-only Web UI for `lazyadmin` that makes dense runtime state easier to scan, sort, filter, and inspect than the default TUI while preserving the current Rust-first architecture and public JSON contracts.

The Web UI should be an observation layer only. It must not expose stop, restart, kill, free-port, run, pause, resume, or other mutating runtime actions in its first version.

## Current Baseline

The repository already has the right data foundation:

- `crates/lazyadmin-core/src/model/mod.rs` defines the public snapshot contract with `Snapshot`, `Listener`, `Process`, `Workload`, `Manager`, `Project`, `TrackedRun`, `Edge`, `Warning`, and `DiscoveryEvent`.
- `crates/lazyadmin-cli/src/main.rs` owns the current adapter assembly in `build_snapshot_with_event_drops`, plus event stream assembly in `event_streams_for_config`.
- `crates/lazyadmin-tui/src/lib.rs` proves that UI-specific view models can be derived from core snapshots without owning discovery or correlation logic.
- `docs/schema/snapshot-v1.md`, `docs/schema/doctor-v1.md`, and `docs/schema/discovery-event-v1.md` document the existing JSON contracts.
- `docs/discovery-events-decision.md` states that discovery events are hints only; snapshot polling remains authoritative.
- `docs/tui.md` records the current TUI projection behavior and search/filter expectations.

Live smoke examples from the current checkout:

```bash
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- doctor --json
cargo run -p lazyadmin-cli -- tui --headless --json
```

The current live host can produce very large snapshots. The Web UI must be designed around hundreds or thousands of listeners/process rows without requiring terminal-width compromises.

## Product Scope

### In Scope

- Local browser UI launched through `lazyadmin web`.
- Read-only runtime overview for listeners, workloads, processes, managers, projects, tracked runs, warnings, and discovery health.
- Dense sortable/filterable tables with stable row identity.
- Inspector panel for selected rows with provenance, warnings, owners, edges, and redacted diagnostic details.
- Saved-in-URL view state for filters, sort, active tab, selected entity, and system-service visibility.
- Snapshot refresh using polling plus optional server-sent discovery-event hints.
- First-class empty, loading, degraded, and permission-limited states.
- Browser smoke tests against fixture data and a live local server.

### Out of Scope For This Plan

- Mutating actions.
- Authentication beyond local-only binding and host checks.
- Multi-host or remote daemon mode.
- Persistent history database.
- Web-based log streaming beyond read-only metadata and future placeholders.
- Replacing the TUI.

## Architecture Decision

Do not put Web UI runtime wiring directly in `crates/lazyadmin-cli/src/main.rs`.

First extract the shared snapshot/event assembly from the CLI into a reusable runtime crate:

- New crate: `crates/lazyadmin-runtime`
- Responsibilities:
  - Load `Config`.
  - Build authoritative `Snapshot` values from procfs, tracked runs, systemd, containers, projects, and portless adapters.
  - Build discovery event streams from enabled adapters.
  - Own the shared `EventDropCounter` path for long-lived runtimes.
  - Expose small async APIs consumed by CLI, TUI, and Web UI.
- Non-responsibilities:
  - Rendering.
  - HTTP.
  - Mutating action execution.
  - Frontend assets.

Then add a Web UI crate:

- New crate: `crates/lazyadmin-web`
- Responsibilities:
  - HTTP server for read-only APIs.
  - Static asset serving for the frontend bundle.
  - Snapshot polling loop and event hint fan-in.
  - Web-specific projections where they reduce frontend complexity.
  - Local-only safety policy.
- Suggested server stack:
  - `axum` for routing.
  - `tower-http` for static assets, tracing, and compression.
  - `tokio` from the workspace.
  - `serde`/`serde_json` from the workspace.
- CLI integration:
  - Add `lazyadmin web` subcommand in `crates/lazyadmin-cli/src/main.rs`.
  - Default bind: `127.0.0.1:0` or `127.0.0.1:7749`.
  - Print the selected URL.
  - Optional flags: `--bind`, `--port`, `--no-open`, `--refresh-ms`.
  - Refuse non-loopback bind unless an explicit unsafe flag is added in a later plan.

## Data Model

The first version should preserve `lazyadmin.snapshot.v1` as the source of truth and add web-specific projections only as convenience views.

### Core API Endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/health` | Server health, version, bind policy, last snapshot status. |
| `GET` | `/api/snapshot` | Full `lazyadmin.snapshot.v1` snapshot. |
| `GET` | `/api/doctor` | `lazyadmin.doctor.v1` style report using the same runtime config. |
| `GET` | `/api/events` | Server-sent events for `lazyadmin.discovery_event.v1` hints. |
| `GET` | `/api/views/overview` | Compact counts and warning rollups. |
| `GET` | `/api/views/listeners` | Table-ready listener rows with owner/runtime/project projections. |
| `GET` | `/api/views/processes` | Table-ready process rows and parent/child metadata. |
| `GET` | `/api/views/workloads` | Table-ready workload rows with listener/process counts. |
| `GET` | `/api/entities/:kind/:id` | Inspector payload for one entity. |

Projection endpoints may be implemented after `/api/snapshot`; the frontend can initially derive views client-side from the full snapshot and then migrate expensive projections server-side.

### Query Parameters

Use the same query shape across view endpoints:

| Param | Values | Notes |
|---|---|---|
| `q` | string | Fuzzy or substring filter over table search text. |
| `sort` | known column key | Invalid keys return `400`. |
| `dir` | `asc` or `desc` | Default depends on the view. |
| `include_system` | boolean | Mirrors TUI `S` behavior. |
| `exposure` | enum or comma list | Useful for public/listening triage. |
| `runtime` | enum or comma list | `direct`, `systemd`, `docker`, `portless`, etc. |
| `warning` | warning code | Filter to rows linked to warnings. |

### Entity Identity

Use existing stable IDs:

- Listeners: `ListenerId`.
- Processes: serialized `ProcessKey` or a URL-safe encoded form of `pid`, `boot_id`, and `start_time_ticks`.
- Workloads/managers/projects/runs: existing IDs.
- Edges: derive from `{kind, from, to}` if a Web UI row identity is needed.

Do not identify processes by PID alone.

## User Experience Specification

### Layout

Use a dense operations dashboard, not a landing page.

- Top bar:
  - Product label.
  - Snapshot age.
  - Host summary.
  - Refresh state.
  - Event drop/degraded indicator.
- Left navigation:
  - Overview.
  - Listeners.
  - Public.
  - Conflicts.
  - Workloads.
  - Processes.
  - Projects.
  - Managers.
  - Tracked runs.
  - Warnings.
  - Discovery health.
- Main pane:
  - High-density data grid.
  - Sticky column headers.
  - Column visibility controls.
  - Multi-column sort where feasible.
  - Keyboard-friendly row selection.
- Right inspector:
  - Selected entity details.
  - Owners and related edges.
  - Provenance.
  - Warnings.
  - Redacted diagnostic summary.

### Primary Views

#### Overview

- Counts: listeners, public listeners, loopback listeners, workloads, processes, managers, projects, tracked runs, warnings.
- Runtime distribution.
- Exposure distribution.
- Warning severity/code distribution.
- Discovery stream status by adapter.

#### Listeners

Default table columns:

- Port.
- Protocol.
- Bind/address or Unix path.
- Exposure.
- Owner.
- Runtime.
- Project.
- Dual-stack state.
- Confidence.
- Warning count.
- Last seen.

Required interactions:

- Sort by port, exposure, owner, runtime, project, confidence, last seen.
- Filter by text, exposure, runtime, warning code, and project.
- Toggle system service visibility.
- Select row to inspect listener, owners, provenance, warnings, and related workload/process.

#### Public

Same data as Listeners, default filtered to non-loopback/non-Unix exposures. Keep public exposure visually distinct but avoid alarmist copy when classification is best-effort.

#### Conflicts

Rows linked to `CONFLICT` warnings or multi-owner listeners. Show the conflict message next to affected rows and make duplicate port/address grouping obvious.

#### Workloads

Columns:

- Name.
- Runtime.
- State.
- Health.
- Process count.
- Listener count.
- Project.
- Manager.
- Restart policy summary.
- Warning count.

#### Processes

Columns:

- PID.
- Command.
- User.
- Runtime.
- Systemd unit.
- Container ID.
- CWD.
- Parent PID.
- Listener count.
- Workload.

Process tree should be a mode of this view or a companion panel, derived from the same `ProcessKey` identity rules as the TUI.

#### Discovery Health

Show:

- Enabled adapters.
- Event source state.
- Drop counts.
- Degraded reasons.
- Snapshot age.
- Last event by adapter.

The copy must reinforce that events are refresh hints and snapshots are authoritative.

## Safety And Security

- Bind to loopback by default.
- Refuse non-loopback bind in v1.
- No write APIs.
- No forms that submit runtime actions.
- No open-url action in v1 unless it is clearly a browser navigation to a loopback listener and remains read-only.
- Use existing redaction defaults.
- Do not expose environment values; only expose `RedactedEnvironmentSummary.keys`.
- Add host/origin checks for API requests even on loopback.
- Keep CORS disabled unless a later plan explicitly adds a safe local integration mode.

## Implementation Tasks

### Milestone 1: Runtime Extraction

- [x] Add `crates/lazyadmin-runtime/Cargo.toml`.
- [x] Move adapter assembly from `crates/lazyadmin-cli/src/main.rs` into runtime functions.
- [x] Move event stream assembly into runtime functions.
- [x] Preserve existing CLI behavior for `export`, `doctor`, `events`, and `tui`.
- [x] Add runtime tests that build an empty or fixture-backed snapshot without launching the TUI.
- [x] Validate that `Snapshot.metadata.events_dropped` still uses the shared counter for long-lived consumers.

### Milestone 2: Web Server Skeleton

- [x] Add `crates/lazyadmin-web/Cargo.toml`.
- [x] Implement local-only HTTP server.
- [x] Add `lazyadmin web` subcommand.
- [x] Serve `/api/health`.
- [x] Serve `/api/snapshot`.
- [x] Serve `/api/doctor`.
- [x] Serve `/api/events` as SSE when event streams are available.
- [x] Return structured JSON errors with stable `code`, `message`, and optional `details`.

### Milestone 3: Frontend Foundation

- [x] Choose the frontend toolchain and document it in the plan follow-up if it adds Node-based build requirements.
- [x] Add an app shell with top bar, nav, main grid, and inspector.
- [x] Load `/api/snapshot` and render real snapshot counts.
- [x] Add loading, empty, degraded, and API error states.
- [x] Store view state in the URL.
- [x] Ensure the app works with static fixture JSON for browser tests.

### Milestone 4: Dense Table Views

- [x] Implement Listeners view.
- [x] Implement Public view as a filtered Listeners view.
- [x] Implement Conflicts view with grouped warnings.
- [x] Implement Workloads view.
- [x] Implement Processes view.
- [x] Implement Warnings view.
- [x] Add column sorting, text filtering, enum filters, and system-service toggle.
- [ ] Use virtualization or pagination before testing against large live snapshots.

### Milestone 5: Inspector And Relationships

- [x] Build entity lookup indexes from snapshot IDs.
- [ ] Resolve owner/workload/manager/project relationships through `edges`.
- [x] Render listener inspector.
- [x] Render process inspector.
- [x] Render workload inspector.
- [ ] Render manager/project/run inspectors.
- [ ] Render provenance in a collapsed-by-default section.
- [ ] Render redacted diagnostic details without raw environment values.

### Milestone 6: Refresh And Discovery Health

- [x] Poll `/api/snapshot` at a configurable interval.
- [x] Subscribe to `/api/events` when available.
- [x] Treat SSE events as refresh hints only.
- [x] Show event drop counts and degraded adapter state.
- [ ] Avoid replacing the current visible table while the user is actively sorting/filtering unless the selected entity disappears.
- [x] Preserve selected entity across refresh using stable IDs.

### Milestone 7: Documentation And Agent Skill

- [ ] Add `docs/webui.md`.
- [ ] Update `README.md` with `lazyadmin web` usage.
- [x] Update `AGENTS.md` with Web UI validation commands and local-only safety caveats.
- [ ] Update `skills/lazyadmin-agent/` only after the command surface is real.
- [ ] Add Web UI JSON/API notes to `docs/agent-integration.md` if agents are expected to use the read-only API.

## Validation Plan

Run after runtime extraction:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- doctor --json
cargo run -p lazyadmin-cli -- events --once --json
cargo run -p lazyadmin-cli -- tui --headless --json
```

Run after Web server skeleton:

```bash
cargo run -p lazyadmin-cli -- web --port 0 --no-open
curl -fsS http://127.0.0.1:<port>/api/health
curl -fsS http://127.0.0.1:<port>/api/snapshot
curl -fsS http://127.0.0.1:<port>/api/doctor
```

Run after frontend implementation:

```bash
cargo test -p lazyadmin-web
cargo clippy -p lazyadmin-web --all-targets -- -D warnings
cargo run -p lazyadmin-cli -- web --port 0 --no-open
```

Add browser verification once the frontend toolchain exists:

- Desktop screenshot at 1440x900.
- Mobile/narrow screenshot at 390px width.
- Dense data fixture with at least 1000 listeners/processes.
- Empty snapshot fixture.
- Degraded adapter fixture.
- Conflicts fixture.

## Acceptance Criteria

- `lazyadmin web` starts a loopback-only local server and prints the URL.
- The Web UI renders without requiring a daemon, root install, or external network access.
- The first screen is the operational dashboard, not a landing page.
- The UI can sort and filter a large listener table without layout breakage.
- Public/conflict views are faster to inspect than the TUI for dense data.
- Selecting a row shows owner, runtime, project, warning, and provenance details.
- Refreshes preserve active filter/sort/selection when possible.
- Discovery events trigger refresh but are not treated as authoritative state.
- No Web UI route can mutate local runtime state.
- Existing CLI/TUI validation remains green.

## Open Questions

- Frontend stack: use a minimal embedded TypeScript app, a Rust/WASM UI, or a server-rendered HTML approach. The current repo has no Node workspace, so adding one should be an explicit decision.
- Static asset packaging: embed assets into the Rust binary for `cargo install`, or serve from a development directory in debug builds and embed only for release.
- API projection boundary: derive all views client-side from `/api/snapshot` first, or add server-side projection endpoints immediately for very large snapshots.
- Browser launch behavior: print URL only in v1, or optionally open the browser when `--open` is passed.
- Visual regression tooling: use Playwright if a Node toolchain is accepted, or keep initial validation to HTTP/API tests plus manual screenshots.
