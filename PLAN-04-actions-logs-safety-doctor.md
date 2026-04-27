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

- [x] Finalize action structs:
  - [x] `Action`,
  - [x] `ActionKind`,
  - [x] `DangerLevel`,
  - [x] `Requirement`,
  - [x] `ConfirmationPolicy`,
  - [x] `ActionPlan`,
  - [x] `ActionResult`,
  - [x] `ActionExecutionReport`.
- [x] Add action executor trait:
  - [x] `plan(target, graph) -> Vec<Action>`.
  - [x] `execute(action, ctx) -> ActionResult`.
- [x] Add common requirements:
  - [x] runtime available,
  - [x] permission/polkit/sudo required,
  - [x] confirmation typed phrase,
  - [x] selector disambiguation,
  - [x] restart policy pause recommended.
- [x] Add dry-run line generation as structured data plus human rendering.
- [x] Add telemetry spans:
  - [x] `action.plan`,
  - [x] `action.execute`,
  - [x] fields: action kind, target, runtime, danger, duration, result, error class.

Tests:

- [x] action serialization.
- [x] confirmation policy rendering.
- [x] dry-run stable output.

## Phase 2 — Runtime-specific action planners

Implement planners before executors.

- [x] Tracked run planner:
  - [x] stop,
  - [x] restart if supported by `PLAN-02` decision,
  - [x] logs,
  - [x] forget.
- [x] systemd planner:
  - [x] stop service/socket,
  - [x] restart service,
  - [x] kill unit as explicit destructive action,
  - [x] pause/resume restart pending spike,
  - [x] logs.
- [x] Docker planner:
  - [x] stop container/compose service,
  - [x] restart,
  - [x] logs,
  - [x] pause/resume restart if update API verified.
- [x] Podman planner:
  - [x] v0.1 read-only logs/list only unless action API verified and tests exist.
  - [x] action attempts should report unsupported with hint, not fall through to signals.
- [x] Direct process planner:
  - [x] SIGTERM process group when safe,
  - [x] SIGTERM PID fallback,
  - [x] SIGKILL only as explicit destructive follow-up,
  - [x] tail-file logs when fd points to regular file only if implemented.

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

- [x] Tracked executor:
  - [x] stop scope/unit/process tree according to `PLAN-02` decision,
  - [x] wait for disappearance,
  - [x] update registry state.
- [x] systemd executor:
  - [x] D-Bus `StopUnit`, `RestartUnit`, `KillUnit` where possible,
  - [x] `systemctl` fallback only with provenance/telemetry marker,
  - [x] respect user vs system bus,
  - [x] surface polkit/sudo failures cleanly.
- [x] Docker executor:
  - [x] bollard stop/restart,
  - [x] timeout handling,
  - [x] distinguish not found/already stopped/permission errors.
- [x] Direct process executor:
  - [x] use `nix` signals,
  - [x] validate `ProcessKey` before signaling,
  - [x] signal process group only when PGID still matches expected owner,
  - [x] wait configurable grace period,
  - [x] never signal unknown PID after validation failure.

Tests:

- [x] direct process SIGTERM safety with PID reuse guard.
- [x] systemd planner chooses manager action over signal.
- [x] Docker planner chooses container stop over docker-proxy process kill.

## Phase 4 — Free-port workflow

Implement command: `lazyadmin free PORT` and TUI action model for later.

Algorithm:

- [x] Resolve exact listener set by selector:
  - [x] protocol,
  - [x] address,
  - [x] namespace,
  - [x] port.
- [x] Resolve all owners for the selected listener(s).
- [x] Build unified action plan covering all owners.
- [x] Render consolidated dry run:
  - [x] every owner to stop,
  - [x] action method per owner,
  - [x] ports expected to disappear,
  - [x] restart policies in effect,
  - [x] what will not be touched.
- [x] Require one confirmation for the whole plan.
- [x] Execute per-owner actions concurrently where independent.
- [x] Collect per-owner result.
- [x] Rescan and run diff against pre-action snapshot.
- [x] Report factual before/after ownership, not success inference.
- [x] If listener remains, show current owner and suggested next actions.

Configuration:

- [x] `actions.free_multi_owner = stop_all | prompt | refuse`.
- [x] typed confirmation phrase defaults from danger level.

Tests:

- [x] single direct process free.
- [x] multi-owner dry run enumerates all owners.
- [x] multi-owner execution reports per-owner partial failure.
- [x] manager auto-restart is reported factually.
- [x] no auto-SIGKILL after listener remains.

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

- [x] Verify Docker `POST /containers/{id}/update` restart policy body and bollard method/type.
- [x] Test changing `always` or `unless-stopped` to `no`.
- [x] Record original policy for resume.
- [x] Verify behavior after stop and after daemon restart if feasible.
- [x] Decide Podman support separately; likely defer actions to v0.2.

### systemd pause spike

Evaluate options:

- [x] `systemctl mask <unit>` + stop.
- [x] Runtime property override via D-Bus `SetUnitProperties` if `Restart` can be changed runtime.
- [x] Transient drop-in or `systemctl edit --runtime` equivalent if appropriate.
- [x] Stop dependent socket unit instead of masking service where socket activation is root cause.

Decision criteria:

- [x] reversible,
- [x] visible to user,
- [x] does not permanently alter user-authored unit files,
- [x] works for user and system units,
- [x] can be represented in doctor,
- [x] safe under permission failure.

Outputs:

- [x] `docs/pause-restart-decision.md`.
- [x] lazyadmin pause registry schema:
  - [x] target selector/entity,
  - [x] runtime,
  - [x] original restart policy/state,
  - [x] operation used,
  - [x] created_at,
  - [x] actor,
  - [x] restore command.

Implementation after decision:

- [x] `lazyadmin pause-restart <selector>`.
- [x] `lazyadmin resume-restart <selector>`.
- [x] Doctor lists paused restart policies.
- [x] Free-port dry run suggests `pause-and-free` when restart policy exists.

Tests:

- [x] pause records original state.
- [x] resume restores original state.
- [x] doctor surfaces paused entries.
- [x] failed pause does not stop target.

## Phase 6 — Log providers

Crate locations:

- core trait in `lazyadmin-core/src/logs/`.
- runtime implementations in adapter crates.

- [x] Define `LogProvider` trait from spec.
- [x] Define `LogStream`, `LogOptions`, `LogLine` with source labels and timestamps.
- [x] systemd logs:
  - [x] journal API or `journalctl` fallback for v0.1,
  - [x] by unit,
  - [x] by PID fallback,
  - [x] by tracked scope/unit if applicable.
- [x] Docker logs:
  - [x] bollard logs API,
  - [x] stdout/stderr labels,
  - [x] follow and tail N.
- [x] Compose logs:
  - [x] group container logs by service,
  - [x] prefix source labels.
- [x] Podman logs:
  - [x] read-only if verified; follow may be v0.2.
- [x] Tracked logs:
  - [x] `journal` or `file` based on proven `TrackedRun.log_source`.
- [x] Direct process:
  - [x] unavailable message by default,
  - [x] optional `tail-file` only when fd/1 or fd/2 resolves to regular file and v0.1 chooses to include it.

CLI:

- [x] `lazyadmin logs <selector>`.
- [x] `--tail N`.
- [x] `--follow`.
- [x] `--json` for structured log lines if feasible.

Tests:

- [x] no managed log source message suggests `lazyadmin run`.
- [x] log provider selection by runtime.
- [x] Docker log stream fixture.
- [x] journalctl command construction safely escapes unit names.

## Phase 7 — Doctor command

Implement `lazyadmin doctor` and `doctor --json`.

- [x] Define `DoctorReport` schema `lazyadmin.doctor.v1`.
- [x] Aggregate health from adapters:
  - [x] OS/kernel,
  - [x] `/proc` readable,
  - [x] `/proc/net` readable,
  - [x] `ss` fallback available,
  - [x] systemd system/user bus,
  - [x] systemd-run/tracked spawn method,
  - [x] journal readability,
  - [x] Docker/Podman socket reachability,
  - [x] Docker socket permission risk,
  - [x] project roots exist,
  - [x] terminal capabilities,
  - [x] clipboard availability,
  - [x] redaction config valid,
  - [x] tracked registry writable,
  - [x] paused restart registry entries.
- [x] Severity levels:
  - [x] ok,
  - [x] info,
  - [x] warning,
  - [x] degraded,
  - [x] error.
- [x] Human output grouped by subsystem.
- [x] JSON output stable and tested.

Tests:

- [x] doctor JSON schema/golden.
- [x] missing Docker socket -> info, not error.
- [x] Docker socket accessible -> warning about root-equivalent control.
- [x] unreadable system journal -> degraded, not fatal.

## Phase 8 — Security/privacy enforcement

- [x] Ensure redaction before:
  - [x] human point query output,
  - [x] copy diagnostic,
  - [x] JSON intended for agent/shareable use,
  - [x] telemetry events,
  - [x] logs metadata.
- [x] Add explicit reveal path with confirmation for local interactive UI only.
- [x] Never recommend adding user to docker group blindly.
- [x] Never chmod Docker/Podman sockets.
- [x] Sudo/polkit escalation must be action-specific and visible in dry run.
- [x] Add security notes to `docs/action-safety.md`.

Tests:

- [x] secrets do not appear in diagnostic copy.
- [x] telemetry event helper redacts sensitive fields.
- [x] Docker permission risk doctor warning.

## Phase 9 — Verification/diff integration

- [x] Before every mutating action, optionally capture pre-action snapshot.
- [x] After action, rescan relevant scope or full snapshot based on action type.
- [x] Run core diff.
- [x] Render factual report:
  - [x] action execution result,
  - [x] before owner,
  - [x] after owner,
  - [x] remaining listeners,
  - [x] restart policy notes,
  - [x] suggested next actions.
- [x] Store reports optionally under `$XDG_STATE_HOME/lazyadmin/action-reports/` if useful; do not make this required for v0.1.

Tests:

- [x] verify reports restarted process PID changed.
- [x] verify does not infer success/failure from rebound listener.
- [x] verify includes partial failures.

## Done criteria

- [x] Every available action is planned with danger, requirements, dry run, confirmation, and provenance.
- [x] `lazyadmin free` handles single and multi-owner ports safely.
- [x] Direct process stop validates `ProcessKey` before signaling.
- [x] Docker and systemd actions use manager APIs before raw signals.
- [x] `pause-restart` has a decision record and implementation or explicit v0.2 deferral where unsafe.
- [x] Logs work for systemd, Docker, tracked runs, and unavailable direct processes with honest messaging.
- [x] Doctor reports adapter health, permissions, and paused restart leftovers.
- [x] Action safety tests cover the acceptance criteria from spec section 25.5.

## Handoff notes for next plan

`PLAN-05` should consume the action planner and doctor/log APIs; the TUI must not execute runtime mutations directly. It should request a plan, render confirmation, then call the same executor used by CLI.
