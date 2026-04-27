# PLAN-05 — Ratatui MVP, Agent Skill, Docs, CI, Packaging, and v0.1 Release

Source: `lazyadmin-spec-v0_2.md` sections 12, 21, 24–28, 31.  
Depends on: `PLAN-01-foundation-core-cli-json.md`, `PLAN-02-discovery-procfs-systemd-tracked.md`, `PLAN-03-container-project-correlation.md`, `PLAN-04-actions-logs-safety-doctor.md`.  
Goal: ship the human-facing TUI, agent integration, documentation, packaging, and release validation for v0.1.

## Phase 1 — TUI crate setup

Crate: `crates/lazyadmin-tui`.

- [x] Add dependencies:
  - [x] `ratatui`,
  - [x] `crossterm`,
  - [x] `tokio`,
  - [x] `tracing`,
  - [x] `color-eyre`,
  - [x] core crate.
- [x] Define app layers:
  - [x] `App` state,
  - [x] `EventLoop`,
  - [x] `SnapshotController`,
  - [x] `CommandDispatcher`,
  - [x] `ViewModel` structs,
  - [x] widgets.
- [x] Keep discovery/action/log calls behind core/adapter services; no TUI-only runtime logic.
- [x] Add terminal panic/restore guard.

Telemetry:

- [x] Spans/events for TUI start/stop, refresh, render duration, input handling, command dispatch, action confirmation/execution.

Validation:

```bash
cargo check -p lazyadmin-tui
```

## Phase 2 — Snapshot refresh architecture

- [x] Separate UI event loop from discovery tasks.
- [x] Add configurable refresh interval from config.
- [x] Use latest completed snapshot; never block keyboard input on adapter timeouts.
- [x] Coalesce refreshes and rate-limit expensive scans.
- [x] Prepare for v0.2 adapter `watch()` events:
  - [x] poll snapshots in v0.1,
  - [x] merge discovery events later without restructuring.
- [x] Add view-model diffing to avoid redrawing everything unnecessarily where practical.

Performance goals:

- [x] initial snapshot under 1s common case,
- [x] warm refresh under 300ms common case,
- [x] TUI input latency under 50ms.

Tests:

- [x] snapshot controller does not block input task.
- [x] adapter timeout surfaces degraded status not frozen UI.

## Phase 3 — Layout and responsive behavior

Implement spec section 12.

- [x] Default three-pane layout:
  - [x] Groups/Filters,
  - [x] Workloads/Listeners,
  - [x] Inspector.
- [x] Responsive thresholds:
  - [x] >=100 columns: three panes,
  - [x] 80–99 columns: inspector collapses into tab accessible with `i`,
  - [x] 60–79 columns: single-pane with `Tab` view switching,
  - [x] <60 columns: refuse TUI with CLI hint.
- [x] Groups pane entries:
  - [x] All/Everything,
  - [x] Ports,
  - [x] Public listeners,
  - [x] Conflicts,
  - [x] Orphans,
  - [x] Tracked runs,
  - [x] Projects,
  - [x] Docker/Compose,
  - [x] Podman,
  - [x] systemd:user,
  - [x] systemd:system hidden state,
  - [x] Direct processes,
  - [x] Logs,
  - [x] Doctor.
- [x] Show hidden system service count whenever two-tier filter hides rows.

Tests:

- [x] view-model golden tests for widths 120/90/70/50.
- [x] hidden count appears when filter active.

## Phase 4 — Views

- [x] Everything view with two-tier filter by default.
- [x] Ports view with protocol/address/netns clarity.
- [x] Projects view grouped by project root/name.
- [x] Managers view grouped by runtime.
- [x] Public view.
- [x] Conflicts view.
- [x] Orphans view.
- [x] Tracked runs view.
- [x] Logs view.
- [x] Doctor view.

For each view:

- [x] Define typed `ViewModel` independent of Ratatui rendering.
- [x] Add snapshot/golden tests for view model output.
- [x] Use warning badges consistently:
  - [x] `PUBLIC`, `CONFLICT`, `ROOT`, `SOCKET-ACT`, `ORPHAN`, `STALE`, `TUNNEL`, `TRACKED`, `RESTARTING`.
- [x] Ensure redacted values only.

## Phase 5 — Inspector panel

Inspector shows:

- [x] identity,
- [x] state,
- [x] runtime,
- [x] ports/listeners,
- [x] process tree,
- [x] project,
- [x] tracked-run metadata,
- [x] restart policy,
- [x] logs preview,
- [x] warnings,
- [x] actions,
- [x] provenance.

Implementation tasks:

- [x] Render provenance as concise expandable list.
- [x] Show confidence next to ownership claims.
- [x] Show restart policy and `pause-restart` hint when applicable.
- [x] Show no-log-source message for raw direct processes.
- [x] Add copy diagnostic action from inspector.

## Phase 6 — Keybindings and command palette

Keybindings from spec:

- [x] `/` fuzzy filter.
- [x] `:` command palette.
- [x] `Tab` / `Shift+Tab` pane navigation.
- [x] `Enter` inspect/expand.
- [x] `l` logs.
- [x] `p` ports for selected entity.
- [x] `t` process tree.
- [x] `r` restart.
- [x] `s` stop.
- [x] `f` free selected port.
- [x] `k` destructive kill after confirmation.
- [x] `o` open local URL.
- [x] `e` edit source config when safe.
- [x] `y` copy diagnostic.
- [x] `S` toggle system services.
- [x] `R` spawn `lazyadmin run` interactively if Phase 02 supports it.
- [x] `?` help.
- [x] `q` quit.

Command palette entries:

- [x] Mirror spec section 12.5.
- [x] Use same action planner as CLI.
- [x] Mutating commands render dry run and confirmation modal.
- [x] Dangerous commands require typed confirmation.

Tests:

- [x] keymap maps each key to expected command.
- [x] command palette filters by fuzzy term.
- [x] action confirmation cannot execute without confirmation.

## Phase 7 — Fuzzy search and open/copy helpers

- [x] Search matches port, bind address, process name, command, cwd, project, container, compose service, systemd unit, image, runtime, tracked tag.
- [x] Search operates on current view-model rows, not raw terminal strings.
- [x] `open` action:
  - [x] enable for localhost TCP listeners on common HTTP ports by default,
  - [x] require explicit config for non-loopback,
  - [x] use safe opener crate or command with careful escaping.
- [x] `copy-diagnostic`:
  - [x] Markdown compact output,
  - [x] redacted,
  - [x] provenance included,
  - [x] clipboard fallback messaging.

## Phase 8 — Agent skill

Create `skills/lazyadmin-agent/`.

Files:

- [x] `SKILL.md` with trigger description from spec section 31.3.
- [x] `always-do-this.md` with behavior rules, adjusted to actual v0.1 commands.
- [x] `cheatsheet.md` compact command reference.
- [x] `json-schema-v1.md` with stable fields and `jq` recipes.
- [x] `examples/01-start-dev-server.md`.
- [x] `examples/02-port-conflict.md`.
- [x] `examples/03-find-my-process.md`.
- [x] `examples/04-tail-logs.md`.
- [x] `examples/05-snapshot-diff.md`.
- [x] `examples/06-fallback-no-lazyadmin.md`.

Critical content requirements:

- [x] Agents check `command -v lazyadmin && lazyadmin doctor --json` once per session.
- [x] Agents wrap long-running commands with `lazyadmin run --tag ... --detach -- <cmd>` only if the `PLAN-02` spawn decision supports it.
- [x] Agents never use `kill $(lsof -ti :PORT)`, `fuser -k`, or `pkill -f` as first-line behavior.
- [x] Agents prefer JSON output and do not parse human output.
- [x] Agents capture snapshot diffs around runtime mutations.
- [x] Agents stop their own tagged runs unless user asked to keep them.
- [x] Skill includes fallback guidance when lazyadmin absent/unhealthy.

Packaging:

- [x] Release artifact `lazyadmin-agent-skill-v<version>.tar.gz`.
- [x] Install script for common skill directories.
- [x] CI verifies tarball contains all required files.

## Phase 9 — Documentation

Docs to create/update:

- [x] `docs/spec.md` — canonical spec copy/link.
- [x] `docs/adapter-protocol.md` — adapter outputs, provenance, health, event hooks.
- [x] `docs/action-safety.md` — danger levels, confirmation, dry-run, pause-restart semantics, sudo/polkit posture.
- [x] `docs/troubleshooting.md` — common permission/runtime issues.
- [x] `docs/agent-integration.md` — mirrors skill but for humans.
- [x] `docs/tracked-run-spawn-decision.md` from `PLAN-02`.
- [x] `docs/pause-restart-decision.md` from `PLAN-04`.
- [x] `docs/schema/snapshot-v1.md`, `diff-v1.md`, `doctor-v1.md`.
- [x] `README.md` quickstart:
  - [x] install,
  - [x] `lazyadmin doctor`,
  - [x] `lazyadmin :3000`,
  - [x] `lazyadmin run`,
  - [x] `lazyadmin free`,
  - [x] TUI.

Docs validation:

- [x] Commands in docs are exercised by a lightweight smoke script where practical.
- [x] Docs explicitly note Linux-first scope and Podman v0.1 read-only status.

## Phase 10 — CI and test matrix

Required CI jobs:

- [x] format,
- [x] clippy,
- [x] unit tests,
- [x] fixture tests,
- [x] JSON schema/golden tests,
- [x] action safety tests,
- [x] TUI view-model tests,
- [x] docs link/command sanity where practical,
- [x] package build.

Linux integration jobs:

- [x] Basic process/socket tests no Docker/systemd dependency.
- [x] Docker tests when Docker service available.
- [x] systemd user tests where CI runner supports it; otherwise run in a VM/container image designed for systemd or keep as documented manual release gate.
- [x] Podman rootless smoke if feasible; skip with explicit reason if not.

Artifacts on failure:

- [x] logs,
- [x] generated snapshots/diffs,
- [x] doctor JSON,
- [x] action reports.

## Phase 11 — Packaging

Targets:

- [x] crates.io-compatible `cargo install lazyadmin`.
- [x] GitHub release Linux x86_64 binary.
- [x] GitHub release Linux aarch64 binary.
- [x] Nix flake if feasible for v0.1; otherwise document v0.2 deferral.
- [x] agent skill tarball.

Tasks:

- [x] Decide crate/package ownership and reserve name if needed.
- [x] Add license files.
- [x] Add versioning policy.
- [x] Add release workflow using `cargo-dist`, `cross`, or equivalent after evaluation.
- [x] Generate shell completions if cheap.
- [x] Ensure binary has no mandatory root install or daemon.
- [x] Smoke-test release binary on a clean Linux VM/container.

## Phase 12 — v0.1 acceptance validation

Validate every acceptance item from spec section 28.

- [x] `lazyadmin :3000` explains direct TCP listener with full provenance.
- [x] `lazyadmin :5432` explains Compose-published Postgres and restart policy.
- [x] systemd user and system services shown distinctly.
- [x] systemd socket activation represented without live service PID.
- [x] public/non-loopback listeners easy to find.
- [x] Compose service can be stopped from TUI with confirmation.
- [x] direct process can be terminated with SIGTERM and verified.
- [x] multiple owners on one port never collapse into fake owner.
- [x] `free 5432` with three owners stops all three atomically and reports per-owner result.
- [x] permission-denied information visible.
- [x] JSON export includes listeners, processes, workloads, managers, projects, tracked runs, edges, provenance.
- [x] JSON diff produces meaningful before/after.
- [x] Doctor gives actionable adapter/permission status with structured severity.
- [x] Secret-looking env/cmdline values and URL userinfo are redacted by default.
- [x] `lazyadmin run` wraps command, graph shows it, and run stop terminates descendants.
- [x] Verify after free-port reports auto-restart factually.
- [x] Two-tier filter hides system-bus units by default; point queries bypass filter.
- [x] Bracketed IPv6 selector parses: `lazyadmin "tcp/[::1]:3000"`.
- [x] lazyadmin-agent skill ships and installs cleanly.

For each item:

- [x] record command/test name,
- [x] record environment,
- [x] save `doctor --json`,
- [x] save before/after snapshots where applicable.

## Done criteria

- [x] TUI is usable at target widths and does not block on discovery.
- [x] TUI actions use the same planner/executor as CLI.
- [x] Agent skill matches the actual shipped CLI/JSON behavior.
- [x] Docs and schema references are consistent.
- [x] CI gates all non-negotiable quality bars.
- [x] Release artifacts are produced and smoke-tested.
- [x] v0.1 acceptance checklist is fully passing or documented with explicit approved deferrals.

## Post-v0.1 follow-up parking lot

Keep these out of v0.1 unless completed early without destabilizing release:

- [ ] Podman actions/log follow.
- [ ] Podman pods UI.
- [ ] sock_diag optimized discovery.
- [ ] manual cgroup fallback for tracked runs.
- [ ] richer sd-journal integration.
- [ ] process tree visualization beyond MVP inspector.
- [ ] metrics panel.
- [ ] live container/systemd event streams.
- [ ] configurable keybindings/themes beyond minimal defaults.
- [ ] Homebrew tap.
- [ ] direct-process tail-file logs if deferred.
