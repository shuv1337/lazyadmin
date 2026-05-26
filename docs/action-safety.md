# Action safety

Lazyadmin actions are planned before execution. Every mutating action must carry a danger level, requirements, dry-run text, confirmation policy, timeout, structured result, and post-action verification where applicable.

## Danger levels

- `safe`: view-only actions such as logs or diagnostic rendering.
- `warn`: restart, normal stop, or pause-restart where a manager can reverse the change.
- `destructive`: raw signals, stopping system services, or explicit kill operations.

`SIGKILL` is never automatic escalation. It may only appear as an explicit follow-up action requested by the user.

## Confirmation

Low-risk actions may use yes/no confirmation. High-risk or multi-owner actions require a typed phrase such as `free` or the unit name. `lazyadmin free PORT` uses one consolidated confirmation for the whole plan, not one prompt per owner. The hidden `--yes-for-test-only` flag exists only for automated tests and emits a warning when used.

## Free-port behavior

Free-port resolves listeners and validates direct process owners before signaling. The current executor can send SIGTERM to a process group or PID after the `ProcessKey` still matches, then rescans and reports the factual result. Manager-aware plans for Compose services, Docker containers, systemd services/sockets, and lazyadmin tracked runs are deferred until their verified executors are wired in; raw process signaling remains the conservative fallback, and SIGKILL is never automatic.

After execution lazyadmin rescans and reports factual before/after state. A rebound listener may indicate a restart policy; lazyadmin reports it rather than claiming success or failure.

Pure free-port planning lives in `lazyadmin-core::actions` so the CLI, TUI, Web, and agents can share the same dry-run facts. Execution remains outside core: the CLI/runtime layer owns live rescans, process-key revalidation, signaling, and manager API calls because those steps depend on host state and adapter availability. Portless planning is deliberately conservative: it targets the portless CLI process key, does not mutate route state directly, and does not call `portless prune`.

## PID reuse guard

Direct process actions validate `ProcessKey` (PID + boot ID + start time) immediately before signaling. If the key does not match, lazyadmin refuses to signal the PID.

## pause-restart semantics

Pause records original restart state in `$XDG_STATE_HOME/lazyadmin/pauses`. Docker/systemd manager mutation is only performed by verified manager executors. Systemd prefers runtime `Restart=no` override; mask+stop is an explicit fallback only.

## sudo/polkit posture

Lazyadmin runs unprivileged by default. Permission escalation must be action-specific, visible in the dry run, and reflected in requirements. Do not recommend `chmod` on Docker/Podman sockets or blindly adding users to the Docker group; accessible Docker socket is a root-equivalent risk and doctor warns about it.

## Privacy

Redaction applies before diagnostic copies, shareable JSON, telemetry, and log metadata. Secret-looking command/environment fields and URL userinfo must be redacted by default. Explicit reveal is reserved for local interactive UI flows with confirmation.
