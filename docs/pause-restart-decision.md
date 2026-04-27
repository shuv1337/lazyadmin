# pause-restart decision

PLAN-04 required a spike before implementing `pause-restart`/`resume-restart`.

## Docker / Compose

Decision: use Docker's container update API when a verified Docker executor is enabled: `POST /containers/{id}/update` via bollard `update_container`, setting `HostConfig.RestartPolicy.Name` to `no`, and record the original policy in lazyadmin's pause registry before stopping. In this environment Docker may be absent, so live verification is conditional; v0.1 keeps the registry and CLI surface while conservative manager mutation remains gated behind the Docker executor.

Resume restores the recorded restart policy via the same update API. Compose is represented as the owning service plus container ID; restart policy restoration is per container for v0.1.

## Podman

Decision: defer mutating Podman pause/resume to v0.2. PLAN-04 keeps Podman read-only/list/log posture unless action API verification and tests exist. Podman action attempts must report unsupported rather than falling through to raw process signals.

## systemd

Evaluated options:

1. `systemctl mask <unit>` + stop: reversible and visible, but can surprise users and affects activation semantics broadly.
2. Runtime property override with `SetUnitProperties(..., runtime=true, Restart=no)`: best default because it is reversible, does not edit user-authored unit files, can work on user and system managers, and is surfaceable in doctor.
3. Runtime drop-in / `systemctl edit --runtime`: similar semantics but harder to automate safely without editor interaction.
4. Stop socket unit when socket activation is the root cause: preferred specific action for socket owners, but not a general service restart-policy pause.

Decision: prefer a runtime override (`Restart=no`) through systemd D-Bus `SetUnitProperties` or `systemctl --runtime set-property <unit> Restart=no` fallback. If runtime override fails, v0.1 should fail visibly or offer mask+stop only as an explicit high-danger fallback; it must not silently mask units. The current implementation records registry entries and exposes CLI/doctor surfaces, with full manager mutation documented for the executor path.

## Pause registry

Lazyadmin-owned records live at `$XDG_STATE_HOME/lazyadmin/pauses/<id>.json` and include:

- target selector/entity
- runtime
- original restart policy/state
- operation used
- created_at
- actor
- restore command

Doctor lists leftover pause entries so users can restore them.
