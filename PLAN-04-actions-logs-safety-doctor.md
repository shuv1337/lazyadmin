# PLAN-04 — Actions, Logs, Safety, Verification, and Doctor

Source: `lazyadmin-spec-v0_2.md` sections 13–19, 20, 23–25, 27–28, 32.  
Depends on: `PLAN-01-foundation-core-cli-json.md`, `PLAN-02-discovery-procfs-systemd-tracked.md`, `PLAN-03-container-project-correlation.md`.  
Goal: implement manager-aware action planning/execution, free-port, pause/resume restart after spikes, log providers, privacy/safety controls, and doctor output.

## Non-negotiable safety rules

- Every mutating action needs a dry run, danger level, confirmation policy, timeout, structured result, and post-action verification when applicable.
- Manager-aware action wins over raw signal when confidence is high.
- `SIGKILL` is never automatic escalation.
- Multi-owner free-port defaults to stop-all only after consolidated confirmation.
- Redaction applies before diagnostics or shareable JSON.

## Phase 1 — Action model and planner foundation

Crate: `crates/lazyadmin-core/src/actions/`.

- [ ] Finalize action structs:
  - [ ] `Action`,
  - [ ] `ActionKind`,
  - [ ] `DangerLevel`,
  - [ ] `Requirement`,
  - [ ] `ConfirmationPolicy`,
  - [ ] `ActionPlan`,
  - [ ] `ActionResult`,
  - [ ] `ActionExecutionReport`.
- [ ] Add action executor trait:
  - [ ] `plan(target, graph) -> Vec<Action>`.
  - [ ] `execute(action, ctx) -> ActionResult`.
- [ ] Add common requirements:
  - [ ] runtime available,
  - [ ] permission/polkit/sudo required,
  - [ ] confirmation typed phrase,
  - [ ] selector disambiguation,
  - [ ] restart policy pause recommended.
- [ ] Add dry-run line generation as structured data plus human rendering.
- [ ] Add telemetry spans:
  - [ ] `action.plan`,
  - [ ] `action.execute`,
  - [ ] fields: action kind, target, runtime, danger, duration, result, error class.

Tests:

- [ ] action serialization.
- [ ] confirmation policy rendering.
- [ ] dry-run stable output.

## Phase 2 — Runtime-specific action planners

Implement planners before executors.

- [ ] Tracked run planner:
  - [ ] stop,
  - [ ] restart if supported by `PLAN-02` decision,
  - [ ] logs,
  - [ ] forget.
- [ ] systemd planner:
  - [ ] stop service/socket,
  - [ ] restart service,
  - [ ] kill unit as explicit destructive action,
  - [ ] pause/resume restart pending spike,
  - [ ] logs.
- [ ] Docker planner:
  - [ ] stop container/compose service,
  - [ ] restart,
  - [ ] logs,
  - [ ] pause/resume restart if update API verified.
- [ ] Podman planner:
  - [ ] v0.1 read-only logs/list only unless action API verified and tests exist.
  - [ ] action attempts should report unsupported with hint, not fall through to signals.
- [ ] Direct process planner:
  - [ ] SIGTERM process group when safe,
  - [ ] SIGTERM PID fallback,
  - [ ] SIGKILL only as explicit destructive follow-up,
  - [ ] tail-file logs when fd points to regular file only if implemented.

Priority order:

1. Compose service stop.
2. Docker container stop.
3. Podman container stop only post-v0.1/if verified.
4. systemd StopUnit.
5. systemd socket StopUnit.
6. lazyadmin run stop.
7. signal process group.
8. signal PID.
9. SIGKILL explicit escalation.

Note: if a workload is both `LazyadminTracked` and a systemd scope, use tracked-run action because it carries registry/log/tag semantics.

## Phase 3 — Action executors

- [ ] Tracked executor:
  - [ ] stop scope/unit/process tree according to `PLAN-02` decision,
  - [ ] wait for disappearance,
  - [ ] update registry state.
- [ ] systemd executor:
  - [ ] D-Bus `StopUnit`, `RestartUnit`, `KillUnit` where possible,
  - [ ] `systemctl` fallback only with provenance/telemetry marker,
  - [ ] respect user vs system bus,
  - [ ] surface polkit/sudo failures cleanly.
- [ ] Docker executor:
  - [ ] bollard stop/restart,
  - [ ] timeout handling,
  - [ ] distinguish not found/already stopped/permission errors.
- [ ] Direct process executor:
  - [ ] use `nix` signals,
  - [ ] validate `ProcessKey` before signaling,
  - [ ] signal process group only when PGID still matches expected owner,
  - [ ] wait configurable grace period,
  - [ ] never signal unknown PID after validation failure.

Tests:

- [ ] direct process SIGTERM safety with PID reuse guard.
- [ ] systemd planner chooses manager action over signal.
- [ ] Docker planner chooses container stop over docker-proxy process kill.

## Phase 4 — Free-port workflow

Implement command: `lazyadmin free PORT` and TUI action model for later.

Algorithm:

- [ ] Resolve exact listener set by selector:
  - [ ] protocol,
  - [ ] address,
  - [ ] namespace,
  - [ ] port.
- [ ] Resolve all owners for the selected listener(s).
- [ ] Build unified action plan covering all owners.
- [ ] Render consolidated dry run:
  - [ ] every owner to stop,
  - [ ] action method per owner,
  - [ ] ports expected to disappear,
  - [ ] restart policies in effect,
  - [ ] what will not be touched.
- [ ] Require one confirmation for the whole plan.
- [ ] Execute per-owner actions concurrently where independent.
- [ ] Collect per-owner result.
- [ ] Rescan and run diff against pre-action snapshot.
- [ ] Report factual before/after ownership, not success inference.
- [ ] If listener remains, show current owner and suggested next actions.

Configuration:

- [ ] `actions.free_multi_owner = stop_all | prompt | refuse`.
- [ ] typed confirmation phrase defaults from danger level.

Tests:

- [ ] single direct process free.
- [ ] multi-owner dry run enumerates all owners.
- [ ] multi-owner execution reports per-owner partial failure.
- [ ] manager auto-restart is reported factually.
- [ ] no auto-SIGKILL after listener remains.

Validation smoke:

```bash
python3 -m http.server 3020 >/tmp/la-free.log 2>&1 & echo $! >/tmp/la-free.pid
cargo run -p lazyadmin-cli -- free 3020 --dry-run
cargo run -p lazyadmin-cli -- free 3020 --yes-for-test-only
```

`--yes-for-test-only` should be hidden or test-only; do not expose unsafe bypass as normal UX without careful naming/config.

## Phase 5 — `pause-restart` / `resume-restart` spike

User confirmed this must be a spike before final choice.

### Container pause spike

- [ ] Verify Docker `POST /containers/{id}/update` restart policy body and bollard method/type.
- [ ] Test changing `always` or `unless-stopped` to `no`.
- [ ] Record original policy for resume.
- [ ] Verify behavior after stop and after daemon restart if feasible.
- [ ] Decide Podman support separately; likely defer actions to v0.2.

### systemd pause spike

Evaluate options:

- [ ] `systemctl mask <unit>` + stop.
- [ ] Runtime property override via D-Bus `SetUnitProperties` if `Restart` can be changed runtime.
- [ ] Transient drop-in or `systemctl edit --runtime` equivalent if appropriate.
- [ ] Stop dependent socket unit instead of masking service where socket activation is root cause.

Decision criteria:

- [ ] reversible,
- [ ] visible to user,
- [ ] does not permanently alter user-authored unit files,
- [ ] works for user and system units,
- [ ] can be represented in doctor,
- [ ] safe under permission failure.

Outputs:

- [ ] `docs/pause-restart-decision.md`.
- [ ] lazyadmin pause registry schema:
  - [ ] target selector/entity,
  - [ ] runtime,
  - [ ] original restart policy/state,
  - [ ] operation used,
  - [ ] created_at,
  - [ ] actor,
  - [ ] restore command.

Implementation after decision:

- [ ] `lazyadmin pause-restart <selector>`.
- [ ] `lazyadmin resume-restart <selector>`.
- [ ] Doctor lists paused restart policies.
- [ ] Free-port dry run suggests `pause-and-free` when restart policy exists.

Tests:

- [ ] pause records original state.
- [ ] resume restores original state.
- [ ] doctor surfaces paused entries.
- [ ] failed pause does not stop target.

## Phase 6 — Log providers

Crate locations:

- core trait in `lazyadmin-core/src/logs/`.
- runtime implementations in adapter crates.

- [ ] Define `LogProvider` trait from spec.
- [ ] Define `LogStream`, `LogOptions`, `LogLine` with source labels and timestamps.
- [ ] systemd logs:
  - [ ] journal API or `journalctl` fallback for v0.1,
  - [ ] by unit,
  - [ ] by PID fallback,
  - [ ] by tracked scope/unit if applicable.
- [ ] Docker logs:
  - [ ] bollard logs API,
  - [ ] stdout/stderr labels,
  - [ ] follow and tail N.
- [ ] Compose logs:
  - [ ] group container logs by service,
  - [ ] prefix source labels.
- [ ] Podman logs:
  - [ ] read-only if verified; follow may be v0.2.
- [ ] Tracked logs:
  - [ ] `journal` or `file` based on proven `TrackedRun.log_source`.
- [ ] Direct process:
  - [ ] unavailable message by default,
  - [ ] optional `tail-file` only when fd/1 or fd/2 resolves to regular file and v0.1 chooses to include it.

CLI:

- [ ] `lazyadmin logs <selector>`.
- [ ] `--tail N`.
- [ ] `--follow`.
- [ ] `--json` for structured log lines if feasible.

Tests:

- [ ] no managed log source message suggests `lazyadmin run`.
- [ ] log provider selection by runtime.
- [ ] Docker log stream fixture.
- [ ] journalctl command construction safely escapes unit names.

## Phase 7 — Doctor command

Implement `lazyadmin doctor` and `doctor --json`.

- [ ] Define `DoctorReport` schema `lazyadmin.doctor.v1`.
- [ ] Aggregate health from adapters:
  - [ ] OS/kernel,
  - [ ] `/proc` readable,
  - [ ] `/proc/net` readable,
  - [ ] `ss` fallback available,
  - [ ] systemd system/user bus,
  - [ ] systemd-run/tracked spawn method,
  - [ ] journal readability,
  - [ ] Docker/Podman socket reachability,
  - [ ] Docker socket permission risk,
  - [ ] project roots exist,
  - [ ] terminal capabilities,
  - [ ] clipboard availability,
  - [ ] redaction config valid,
  - [ ] tracked registry writable,
  - [ ] paused restart registry entries.
- [ ] Severity levels:
  - [ ] ok,
  - [ ] info,
  - [ ] warning,
  - [ ] degraded,
  - [ ] error.
- [ ] Human output grouped by subsystem.
- [ ] JSON output stable and tested.

Tests:

- [ ] doctor JSON schema/golden.
- [ ] missing Docker socket -> info, not error.
- [ ] Docker socket accessible -> warning about root-equivalent control.
- [ ] unreadable system journal -> degraded, not fatal.

## Phase 8 — Security/privacy enforcement

- [ ] Ensure redaction before:
  - [ ] human point query output,
  - [ ] copy diagnostic,
  - [ ] JSON intended for agent/shareable use,
  - [ ] telemetry events,
  - [ ] logs metadata.
- [ ] Add explicit reveal path with confirmation for local interactive UI only.
- [ ] Never recommend adding user to docker group blindly.
- [ ] Never chmod Docker/Podman sockets.
- [ ] Sudo/polkit escalation must be action-specific and visible in dry run.
- [ ] Add security notes to `docs/action-safety.md`.

Tests:

- [ ] secrets do not appear in diagnostic copy.
- [ ] telemetry event helper redacts sensitive fields.
- [ ] Docker permission risk doctor warning.

## Phase 9 — Verification/diff integration

- [ ] Before every mutating action, optionally capture pre-action snapshot.
- [ ] After action, rescan relevant scope or full snapshot based on action type.
- [ ] Run core diff.
- [ ] Render factual report:
  - [ ] action execution result,
  - [ ] before owner,
  - [ ] after owner,
  - [ ] remaining listeners,
  - [ ] restart policy notes,
  - [ ] suggested next actions.
- [ ] Store reports optionally under `$XDG_STATE_HOME/lazyadmin/action-reports/` if useful; do not make this required for v0.1.

Tests:

- [ ] verify reports restarted process PID changed.
- [ ] verify does not infer success/failure from rebound listener.
- [ ] verify includes partial failures.

## Done criteria

- [ ] Every available action is planned with danger, requirements, dry run, confirmation, and provenance.
- [ ] `lazyadmin free` handles single and multi-owner ports safely.
- [ ] Direct process stop validates `ProcessKey` before signaling.
- [ ] Docker and systemd actions use manager APIs before raw signals.
- [ ] `pause-restart` has a decision record and implementation or explicit v0.2 deferral where unsafe.
- [ ] Logs work for systemd, Docker, tracked runs, and unavailable direct processes with honest messaging.
- [ ] Doctor reports adapter health, permissions, and paused restart leftovers.
- [ ] Action safety tests cover the acceptance criteria from spec section 25.5.

## Handoff notes for next plan

`PLAN-05` should consume the action planner and doctor/log APIs; the TUI must not execute runtime mutations directly. It should request a plan, render confirmation, then call the same executor used by CLI.
