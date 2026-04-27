# PLAN-00 — Implementation Index and Assumption Review

Source spec: `lazyadmin-spec-v0_2.md`  
Date: 2026-04-27  
Status: Implementation-ready planning set, with explicit spikes for unresolved assumptions.

## Scope decisions confirmed

- [x] Target v0.1 MVP with cheap v0.2 architectural hooks.
- [x] Initialize this repo as a Rust workspace from scratch.
- [x] Keep `systemd-run --user --scope` as the v0.1 tracked-run candidate, but validate/downgrade detached-run and logging claims before committing APIs.
- [x] Treat systemd `pause-restart` as a spike before choosing mask vs runtime override semantics.
- [x] Non-negotiable release gates:
  - Linux CI with fixture/integration tests.
  - Stable JSON schema and compatibility tests.
  - Action safety, dry-run, confirmation, and post-action verification tests.
  - Agent skill shipped as release artifact.
  - GitHub release binaries plus `cargo install` path.

## Plan document map

Implement in this order:

1. `PLAN-01-foundation-core-cli-json.md`
   - Workspace scaffolding, models, IDs, config, redaction, selector parser, JSON schemas, diff, telemetry foundation, CLI skeleton.
2. `PLAN-02-discovery-procfs-systemd-tracked.md`
   - `/proc/net`, process enrichment, socket-to-PID mapping, systemd correlation, socket units, tracked-run registry, `lazyadmin run` spike and MVP behavior.
3. `PLAN-03-container-project-correlation.md`
   - Docker/Podman discovery via Docker-compatible APIs, Compose grouping, project detection, conflict handling, workload/listener graph correlation.
4. `PLAN-04-actions-logs-safety-doctor.md`
   - Action planning/execution, free-port, pause-restart spike, logs, doctor, safety/privacy behavior.
5. `PLAN-05-tui-agent-packaging-release.md`
   - Ratatui MVP, agent skill, docs, CI, release packaging, acceptance validation.

## Assumption validation summary

### Confirmed or mostly confirmed

- [x] **systemd D-Bus Manager interface exists for required operations.** The documented `org.freedesktop.systemd1.Manager` interface includes `GetUnitByPID`, `GetUnitByPIDFD`, `GetUnitByControlGroup`, `ListUnits`, `StartUnit`, `StopUnit`, `RestartUnit`, `KillUnit`, `StartTransientUnit`, and related job methods. Implementation must still feature-detect at runtime because `GetUnitByPIDFD` depends on systemd/kernel support.
- [x] **`systemd-run` supports transient scopes and user mode.** The man page documents `--scope`, `--user`, `--unit`, `--property`, `--collect`, and examples using user scopes.
- [x] **Bollard is a suitable Docker Engine API client.** Current docs identify it as an async Rust client for the Docker API with Unix socket, TCP, HTTPS, version negotiation, container listing/logs/stats APIs, and generated Docker API 1.52 types.
- [x] **`/proc/net` is an acceptable v0.1 primary socket source.** It exposes listener/socket inode data needed for inode-to-FD correlation. The plan includes a fallback parser path if `procfs` crate coverage is insufficient.

### Assumptions requiring spikes before implementation locks in

- [ ] **Tracked-run detach/logging semantics.** `systemd-run --scope` is synchronous and runs the command as a child of `systemd-run`, inheriting the caller environment. The spec claims it solves historical stdout via journal and supports `--detach`. That is not proven. Spike outcomes allowed:
  1. Keep scopes with a tiny lazyadmin shim process that owns log capture/registry and can detach safely.
  2. Use transient user services for detached runs and scopes only for foreground/interactive runs.
  3. Keep scopes but explicitly document no guaranteed historical logs unless stdout/stderr are redirected by the shim.
- [ ] **Bollard + Podman details.** Do not assume automatic Podman socket discovery from bollard. Implement explicit socket probing and prove Docker-compatible Podman endpoints work for list/inspect/logs; gate Libpod-specific pod features behind v0.2.
- [ ] **Docker/Podman restart-policy update shape.** Verify exact request body support for `POST /containers/{id}/update` before implementing `pause-restart` for containers.
- [ ] **systemd pause-restart semantics.** Masking a unit is reversible but heavy-handed and not equivalent to editing `Restart=`. Spike runtime `SetUnitProperties`, transient drop-ins, and mask fallback for user/system units.
- [ ] **Exact IPv6 dual-stack detection.** `/proc/net/tcp6` does not obviously expose per-socket `IPV6_V6ONLY`. v0.1 should mark dual-stack warnings as best-effort unless proven through FD inspection or an available API.
- [ ] **systemd socket unit listener extraction.** Confirm the exact zbus property/method shape for reading `ListenStream`, `ListenDatagram`, `Accept`, `Service`, and related socket properties. If D-Bus does not expose enough detail, use `systemctl show`/unit file parsing as marked fallback.

## Plan review summary

The product direction is coherent and implementable, but the original milestone order would carry too much risk if TUI and container work start before the low-level discovery/action contracts are proven. The rewritten plans front-load:

- public JSON schemas and snapshot/diff contract,
- fixture-driven parser/correlation tests,
- tracked-run and pause-restart spikes,
- action dry-run/confirmation safety,
- telemetry spans from the first crate.

## Critical issues found in the spec

### 1. `lazyadmin run` scope behavior conflicts with detach/log claims

Reference: `lazyadmin-spec-v0_2.md` sections 10.7, 15.1, 27.  
Risk: high — tracked runs are a differentiator and agent-safety foundation.

`systemd-run --user --scope` is documented as synchronous: it returns when the command finishes. The command is parented by `systemd-run` and inherits the caller environment. This conflicts with `lazyadmin run --detach` returning immediately and with the claim that journald automatically captures historical stdout for scope-based tracked runs.

Resolution in plans:

- `PLAN-02` begins with a tracked-run spike.
- v0.1 CLI must expose only the behavior proven by the spike.
- JSON schema should support `log_source: journal | file | unavailable | unknown` so docs and agents do not assume logs exist.

### 2. `pause-restart` systemd behavior is underspecified

Reference: `lazyadmin-spec-v0_2.md` section 14.6.  
Risk: high — masking services can surprise users and is not a lightweight temporary restart-policy pause.

Resolution in plans:

- `PLAN-04` treats systemd pause-resume as a safety spike.
- Any implementation needs a lazyadmin-owned pause registry with original state, timestamp, actor, and restore operation.
- Doctor must show paused/masked state prominently.

### 3. Podman support is broader in the spec than in verified library docs

Reference: `lazyadmin-spec-v0_2.md` section 10.4.  
Risk: medium — read-only Podman discovery is still achievable, but claiming first-class automatic support may overpromise.

Resolution in plans:

- `PLAN-03` makes Docker-compatible endpoint probing explicit.
- Podman v0.1 is read-only and skips log follow/actions unless verified.
- The adapter model keeps runtime kind separate from client implementation so Podman can grow later.

### 4. Exact dual-stack listener warnings need proof

Reference: `lazyadmin-spec-v0_2.md` sections 12.9, 20.  
Risk: medium — false confidence in exposure warnings is harmful.

Resolution in plans:

- v0.1 warns on `[::]` and `0.0.0.0` binds accurately.
- The dual-stack-specific badge is `possible-dual-stack` unless `IPV6_V6ONLY` is proven.

## Important implementation constraints

- [ ] Do not place correlation logic in `lazyadmin-cli` or `lazyadmin-tui`; both consume core view models.
- [ ] Every adapter output must include provenance and confidence.
- [ ] Every mutating action must support dry-run, confirmation policy, timeout, structured result, and post-action diff.
- [ ] Treat `schema_version` as a public API and test it.
- [ ] Redaction must run before any value reaches human output, JSON output intended for sharing, logs, or diagnostics.
- [ ] Telemetry spans are required for snapshot phases, adapter health/discovery, correlation, action planning/execution, diff, and CLI/TUI commands.

## Open questions carried forward

These are not blockers for scaffolding, but they must be resolved before affected milestones complete:

- [ ] Should v0.1 support foreground `lazyadmin run` at all, or only `--detach` tracked dev processes?
- [ ] What minimum supported systemd version should CI emulate/document?
- [ ] Which Linux distributions are first-class for v0.1 binary releases?
- [ ] Should the binary/package name be reserved on crates.io before implementation reaches public CI?
- [ ] Should the JSON schema be documented as JSON Schema files under `docs/schema/` or Markdown-only under `docs/`?

## Done criteria for this planning set

- [x] The original spec has been reviewed end-to-end.
- [x] Major risky assumptions are identified with mitigation spikes.
- [x] Implementation docs are split by dependency order.
- [x] Each doc includes validation commands and acceptance gates.
- [x] Local `AGENTS.md` captures current repo state and caveats.
