# PLAN-05 — Ratatui MVP, Agent Skill, Docs, CI, Packaging, and v0.1 Release

Source: `lazyadmin-spec-v0_2.md` sections 12, 21, 24–28, 31.  
Depends on: `PLAN-01-foundation-core-cli-json.md`, `PLAN-02-discovery-procfs-systemd-tracked.md`, `PLAN-03-container-project-correlation.md`, `PLAN-04-actions-logs-safety-doctor.md`.  
Goal: ship the human-facing TUI, agent integration, documentation, packaging, and release validation for v0.1.

## Phase 1 — TUI crate setup

Crate: `crates/lazyadmin-tui`.

- [ ] Add dependencies:
  - [ ] `ratatui`,
  - [ ] `crossterm`,
  - [ ] `tokio`,
  - [ ] `tracing`,
  - [ ] `color-eyre`,
  - [ ] core crate.
- [ ] Define app layers:
  - [ ] `App` state,
  - [ ] `EventLoop`,
  - [ ] `SnapshotController`,
  - [ ] `CommandDispatcher`,
  - [ ] `ViewModel` structs,
  - [ ] widgets.
- [ ] Keep discovery/action/log calls behind core/adapter services; no TUI-only runtime logic.
- [ ] Add terminal panic/restore guard.

Telemetry:

- [ ] Spans/events for TUI start/stop, refresh, render duration, input handling, command dispatch, action confirmation/execution.

Validation:

```bash
cargo check -p lazyadmin-tui
```

## Phase 2 — Snapshot refresh architecture

- [ ] Separate UI event loop from discovery tasks.
- [ ] Add configurable refresh interval from config.
- [ ] Use latest completed snapshot; never block keyboard input on adapter timeouts.
- [ ] Coalesce refreshes and rate-limit expensive scans.
- [ ] Prepare for v0.2 adapter `watch()` events:
  - [ ] poll snapshots in v0.1,
  - [ ] merge discovery events later without restructuring.
- [ ] Add view-model diffing to avoid redrawing everything unnecessarily where practical.

Performance goals:

- [ ] initial snapshot under 1s common case,
- [ ] warm refresh under 300ms common case,
- [ ] TUI input latency under 50ms.

Tests:

- [ ] snapshot controller does not block input task.
- [ ] adapter timeout surfaces degraded status not frozen UI.

## Phase 3 — Layout and responsive behavior

Implement spec section 12.

- [ ] Default three-pane layout:
  - [ ] Groups/Filters,
  - [ ] Workloads/Listeners,
  - [ ] Inspector.
- [ ] Responsive thresholds:
  - [ ] >=100 columns: three panes,
  - [ ] 80–99 columns: inspector collapses into tab accessible with `i`,
  - [ ] 60–79 columns: single-pane with `Tab` view switching,
  - [ ] <60 columns: refuse TUI with CLI hint.
- [ ] Groups pane entries:
  - [ ] All/Everything,
  - [ ] Ports,
  - [ ] Public listeners,
  - [ ] Conflicts,
  - [ ] Orphans,
  - [ ] Tracked runs,
  - [ ] Projects,
  - [ ] Docker/Compose,
  - [ ] Podman,
  - [ ] systemd:user,
  - [ ] systemd:system hidden state,
  - [ ] Direct processes,
  - [ ] Logs,
  - [ ] Doctor.
- [ ] Show hidden system service count whenever two-tier filter hides rows.

Tests:

- [ ] view-model golden tests for widths 120/90/70/50.
- [ ] hidden count appears when filter active.

## Phase 4 — Views

- [ ] Everything view with two-tier filter by default.
- [ ] Ports view with protocol/address/netns clarity.
- [ ] Projects view grouped by project root/name.
- [ ] Managers view grouped by runtime.
- [ ] Public view.
- [ ] Conflicts view.
- [ ] Orphans view.
- [ ] Tracked runs view.
- [ ] Logs view.
- [ ] Doctor view.

For each view:

- [ ] Define typed `ViewModel` independent of Ratatui rendering.
- [ ] Add snapshot/golden tests for view model output.
- [ ] Use warning badges consistently:
  - [ ] `PUBLIC`, `CONFLICT`, `ROOT`, `SOCKET-ACT`, `ORPHAN`, `STALE`, `TUNNEL`, `TRACKED`, `RESTARTING`.
- [ ] Ensure redacted values only.

## Phase 5 — Inspector panel

Inspector shows:

- [ ] identity,
- [ ] state,
- [ ] runtime,
- [ ] ports/listeners,
- [ ] process tree,
- [ ] project,
- [ ] tracked-run metadata,
- [ ] restart policy,
- [ ] logs preview,
- [ ] warnings,
- [ ] actions,
- [ ] provenance.

Implementation tasks:

- [ ] Render provenance as concise expandable list.
- [ ] Show confidence next to ownership claims.
- [ ] Show restart policy and `pause-restart` hint when applicable.
- [ ] Show no-log-source message for raw direct processes.
- [ ] Add copy diagnostic action from inspector.

## Phase 6 — Keybindings and command palette

Keybindings from spec:

- [ ] `/` fuzzy filter.
- [ ] `:` command palette.
- [ ] `Tab` / `Shift+Tab` pane navigation.
- [ ] `Enter` inspect/expand.
- [ ] `l` logs.
- [ ] `p` ports for selected entity.
- [ ] `t` process tree.
- [ ] `r` restart.
- [ ] `s` stop.
- [ ] `f` free selected port.
- [ ] `k` destructive kill after confirmation.
- [ ] `o` open local URL.
- [ ] `e` edit source config when safe.
- [ ] `y` copy diagnostic.
- [ ] `S` toggle system services.
- [ ] `R` spawn `lazyadmin run` interactively if Phase 02 supports it.
- [ ] `?` help.
- [ ] `q` quit.

Command palette entries:

- [ ] Mirror spec section 12.5.
- [ ] Use same action planner as CLI.
- [ ] Mutating commands render dry run and confirmation modal.
- [ ] Dangerous commands require typed confirmation.

Tests:

- [ ] keymap maps each key to expected command.
- [ ] command palette filters by fuzzy term.
- [ ] action confirmation cannot execute without confirmation.

## Phase 7 — Fuzzy search and open/copy helpers

- [ ] Search matches port, bind address, process name, command, cwd, project, container, compose service, systemd unit, image, runtime, tracked tag.
- [ ] Search operates on current view-model rows, not raw terminal strings.
- [ ] `open` action:
  - [ ] enable for localhost TCP listeners on common HTTP ports by default,
  - [ ] require explicit config for non-loopback,
  - [ ] use safe opener crate or command with careful escaping.
- [ ] `copy-diagnostic`:
  - [ ] Markdown compact output,
  - [ ] redacted,
  - [ ] provenance included,
  - [ ] clipboard fallback messaging.

## Phase 8 — Agent skill

Create `skills/lazyadmin-agent/`.

Files:

- [ ] `SKILL.md` with trigger description from spec section 31.3.
- [ ] `always-do-this.md` with behavior rules, adjusted to actual v0.1 commands.
- [ ] `cheatsheet.md` compact command reference.
- [ ] `json-schema-v1.md` with stable fields and `jq` recipes.
- [ ] `examples/01-start-dev-server.md`.
- [ ] `examples/02-port-conflict.md`.
- [ ] `examples/03-find-my-process.md`.
- [ ] `examples/04-tail-logs.md`.
- [ ] `examples/05-snapshot-diff.md`.
- [ ] `examples/06-fallback-no-lazyadmin.md`.

Critical content requirements:

- [ ] Agents check `command -v lazyadmin && lazyadmin doctor --json` once per session.
- [ ] Agents wrap long-running commands with `lazyadmin run --tag ... --detach -- <cmd>` only if the `PLAN-02` spawn decision supports it.
- [ ] Agents never use `kill $(lsof -ti :PORT)`, `fuser -k`, or `pkill -f` as first-line behavior.
- [ ] Agents prefer JSON output and do not parse human output.
- [ ] Agents capture snapshot diffs around runtime mutations.
- [ ] Agents stop their own tagged runs unless user asked to keep them.
- [ ] Skill includes fallback guidance when lazyadmin absent/unhealthy.

Packaging:

- [ ] Release artifact `lazyadmin-agent-skill-v<version>.tar.gz`.
- [ ] Install script for common skill directories.
- [ ] CI verifies tarball contains all required files.

## Phase 9 — Documentation

Docs to create/update:

- [ ] `docs/spec.md` — canonical spec copy/link.
- [ ] `docs/adapter-protocol.md` — adapter outputs, provenance, health, event hooks.
- [ ] `docs/action-safety.md` — danger levels, confirmation, dry-run, pause-restart semantics, sudo/polkit posture.
- [ ] `docs/troubleshooting.md` — common permission/runtime issues.
- [ ] `docs/agent-integration.md` — mirrors skill but for humans.
- [ ] `docs/tracked-run-spawn-decision.md` from `PLAN-02`.
- [ ] `docs/pause-restart-decision.md` from `PLAN-04`.
- [ ] `docs/schema/snapshot-v1.md`, `diff-v1.md`, `doctor-v1.md`.
- [ ] `README.md` quickstart:
  - [ ] install,
  - [ ] `lazyadmin doctor`,
  - [ ] `lazyadmin :3000`,
  - [ ] `lazyadmin run`,
  - [ ] `lazyadmin free`,
  - [ ] TUI.

Docs validation:

- [ ] Commands in docs are exercised by a lightweight smoke script where practical.
- [ ] Docs explicitly note Linux-first scope and Podman v0.1 read-only status.

## Phase 10 — CI and test matrix

Required CI jobs:

- [ ] format,
- [ ] clippy,
- [ ] unit tests,
- [ ] fixture tests,
- [ ] JSON schema/golden tests,
- [ ] action safety tests,
- [ ] TUI view-model tests,
- [ ] docs link/command sanity where practical,
- [ ] package build.

Linux integration jobs:

- [ ] Basic process/socket tests no Docker/systemd dependency.
- [ ] Docker tests when Docker service available.
- [ ] systemd user tests where CI runner supports it; otherwise run in a VM/container image designed for systemd or keep as documented manual release gate.
- [ ] Podman rootless smoke if feasible; skip with explicit reason if not.

Artifacts on failure:

- [ ] logs,
- [ ] generated snapshots/diffs,
- [ ] doctor JSON,
- [ ] action reports.

## Phase 11 — Packaging

Targets:

- [ ] crates.io-compatible `cargo install lazyadmin`.
- [ ] GitHub release Linux x86_64 binary.
- [ ] GitHub release Linux aarch64 binary.
- [ ] Nix flake if feasible for v0.1; otherwise document v0.2 deferral.
- [ ] agent skill tarball.

Tasks:

- [ ] Decide crate/package ownership and reserve name if needed.
- [ ] Add license files.
- [ ] Add versioning policy.
- [ ] Add release workflow using `cargo-dist`, `cross`, or equivalent after evaluation.
- [ ] Generate shell completions if cheap.
- [ ] Ensure binary has no mandatory root install or daemon.
- [ ] Smoke-test release binary on a clean Linux VM/container.

## Phase 12 — v0.1 acceptance validation

Validate every acceptance item from spec section 28.

- [ ] `lazyadmin :3000` explains direct TCP listener with full provenance.
- [ ] `lazyadmin :5432` explains Compose-published Postgres and restart policy.
- [ ] systemd user and system services shown distinctly.
- [ ] systemd socket activation represented without live service PID.
- [ ] public/non-loopback listeners easy to find.
- [ ] Compose service can be stopped from TUI with confirmation.
- [ ] direct process can be terminated with SIGTERM and verified.
- [ ] multiple owners on one port never collapse into fake owner.
- [ ] `free 5432` with three owners stops all three atomically and reports per-owner result.
- [ ] permission-denied information visible.
- [ ] JSON export includes listeners, processes, workloads, managers, projects, tracked runs, edges, provenance.
- [ ] JSON diff produces meaningful before/after.
- [ ] Doctor gives actionable adapter/permission status with structured severity.
- [ ] Secret-looking env/cmdline values and URL userinfo are redacted by default.
- [ ] `lazyadmin run` wraps command, graph shows it, and run stop terminates descendants.
- [ ] Verify after free-port reports auto-restart factually.
- [ ] Two-tier filter hides system-bus units by default; point queries bypass filter.
- [ ] Bracketed IPv6 selector parses: `lazyadmin "tcp/[::1]:3000"`.
- [ ] lazyadmin-agent skill ships and installs cleanly.

For each item:

- [ ] record command/test name,
- [ ] record environment,
- [ ] save `doctor --json`,
- [ ] save before/after snapshots where applicable.

## Done criteria

- [ ] TUI is usable at target widths and does not block on discovery.
- [ ] TUI actions use the same planner/executor as CLI.
- [ ] Agent skill matches the actual shipped CLI/JSON behavior.
- [ ] Docs and schema references are consistent.
- [ ] CI gates all non-negotiable quality bars.
- [ ] Release artifacts are produced and smoke-tested.
- [ ] v0.1 acceptance checklist is fully passing or documented with explicit approved deferrals.

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
