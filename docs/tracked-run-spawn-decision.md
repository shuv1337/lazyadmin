# Tracked-run spawn decision (PLAN-02)

Date: 2026-04-27

## Spike results

Commands run on this host with timeouts:

- `systemd-run --user --scope --collect --unit=lazyadmin-spike-<id>.scope -- sleep 1`
  - Returned after the command completed.
  - Created a user scope and cgroup suitable for discovery while the process was alive.
  - Scope stdout/stderr history is not a reliable lazyadmin log API by itself.
- `systemd-run --user --unit=lazyadmin-spike-<id>.service --collect --property=Type=exec -- sleep 2`
  - Returned immediately after queuing/starting the transient service.
  - `journalctl --user -u <unit>` showed service lifecycle logs on this host.
  - This is better for journal-backed logs, but has different foreground/detach semantics than a scope.

## Decision for PLAN-02 MVP

The public JSON model keeps the external `spawn_method` and `log_source` fields so the implementation can evolve without a schema break.

For the MVP implemented in PLAN-02, `lazyadmin run --detach` uses a conservative hybrid/direct shim behavior:

- spawn method: `direct_detached_file_log`
- log source: `file`
- registry path: `$XDG_RUNTIME_DIR/lazyadmin/runs/<id>.json`
- log path: `$XDG_RUNTIME_DIR/lazyadmin/logs/<id>.log`

This intentionally downgrades the original scope/journal claim: scopes are still the preferred future candidate for process-tree ownership, but they do not satisfy detached execution plus durable stdout/stderr history alone. A future implementation can wrap the command in a lazyadmin shim launched inside `systemd-run --user --scope` and keep `log_source=file`, or use transient user services when `log_source=journal` is explicitly proven for the target environment.

## `log_source` semantics

- `journal`: logs are read from the systemd journal for the recorded scope/unit.
- `file`: stdout/stderr were redirected to a lazyadmin-owned log file recorded in the registry.
- `unavailable`: no safe log source was established for this run.
- `unknown`: legacy or malformed registry entry; lazyadmin cannot assert where logs live.

## Rejected/deferred alternatives

- Pure `systemd-run --user --scope` for `--detach`: deferred because scope invocation is command-lifetime oriented and does not by itself provide a durable log source.
- Pure transient user service: viable for journal logs, but changes process/session semantics and needs more tests for cwd/env, command-not-found reporting, and logout/lingering behavior.
- Direct D-Bus `StartTransientUnit`: deferred until the CLI behavior is pinned down; it should mirror the chosen scope/service behavior rather than introduce a third semantic path.
