# PLAN-11 — v0.2 Discovery Upgrades: sock_diag, IPv6 Dual-Stack Proof, Watch() Event Streams

Source: `lazyadmin-spec-v0_2.md` §6, §10.1, §11, §12.9, §18.2, §20, §21.2, §27.
Depends on: `PLAN-10-v0_2-index-and-assumption-review.md`. Builds on v0.1 (`PLAN-01..05`).
Goal: deepen the discovery layer with (1) a sock_diag socket source as an opt-in alternative to `/proc/net`, (2) per-FD IPv6 dual-stack proof, and (3) real adapter `watch()` event streams (procfs debounced poll, Docker `/events`, systemd `PropertiesChanged`) wired into the orchestrator so the TUI in PLAN-12 can refresh live.

## Implementation principles

- Additive, opt-in, and reversible. Default behavior must match v0.1 unless a user opts into the new path.
- Evidence-preserving: new sources add provenance, never overwrite existing high-confidence evidence silently.
- Fallback first: if the new path errors out, the orchestrator must fall back to v0.1 polling and signal degraded state in doctor.
- JSON contract is additive only. No removed/renamed fields.

## Cross-cutting tasks (do early)

- [ ] Add a `lazyadmin.discovery_event.v1` JSON schema doc under `docs/schema/discovery-event-v1.md` describing event kinds, payloads, and ordering guarantees.
- [ ] Extend `Listener` with optional `dual_stack_state: DualStackState` (`not_applicable | confirmed_dual_stack | confirmed_v6_only | possible | unknown`) — default `unknown` for backward compatibility.
- [ ] Extend `Snapshot.metadata` with optional `events_dropped: u64` for use when watch streams overflow.
- [ ] Add config knobs:
  - [ ] `adapters.sockets.preferred = "proc" | "sock_diag" | "both"` (default `"proc"`).
  - [ ] `adapters.sockets.confirm_dual_stack = bool` (default `true`; setting `false` keeps v0.1 best-effort posture).
  - [ ] `adapters.events.enabled = bool` (default `true`).
  - [ ] `adapters.events.channel_capacity = usize` (default `256`).
- [ ] Add core enum/struct: `DiscoveryEvent { Added(EntityRef), Removed(EntityRef), Changed(EntityRef, Vec<FieldChange>), Heartbeat, Degraded { adapter, reason } }`.
- [ ] Extend `DoctorReport.subsystems` with `events` block reporting per-adapter watch state and any `events_dropped` count.
- [ ] Telemetry span names registered in `lazyadmin-core/src/telemetry/`: `adapter.watch.start`, `adapter.watch.event`, `adapter.watch.stop`, `adapter.sockdiag.discover`, `listener.dualstack.probe`.

Validation:

```bash
cargo test -p lazyadmin-core schema discovery_event
cargo run -p lazyadmin-cli -- export --json | jq '.metadata.events_dropped // 0'
```

## Phase 1 — sock_diag adapter spike

Spike must complete before Phase 2 ships an opt-in default.

- [ ] Evaluate dependency choice:
  - [ ] `netlink-packet-sock-diag` + `netlink-packet-core` + `netlink-sys` (preferred if maintenance is acceptable).
  - [ ] Hand-rolled minimal client using `socket2` and `nix` if the crates are too stale or bug-prone.
  - [ ] Decision recorded in `docs/sock-diag-decision.md` with rationale.
- [ ] Build a parity test harness:
  - [ ] Inputs: every fixture used by `lazyadmin-adapter-procfs` plus several live local listeners (Python http.server, simple UDP echo, Unix socket).
  - [ ] Output: side-by-side `Listener` lists.
  - [ ] Assertions: identical (proto, family, bind_addr, port, exposure) for every entry; socket inode equality where applicable.
- [ ] Document permission posture: sock_diag requires either `CAP_NET_ADMIN` for some queries or runs unprivileged for the basic listening enumeration. Capture exact failure modes per kernel/distro mix.

Acceptance:

- [ ] Decision doc committed.
- [ ] Parity harness can run as `cargo test -p lazyadmin-adapter-procfs --features sock_diag sock_diag_parity -- --include-ignored` (with explicit `#[ignore]` for live-only cases).

## Phase 2 — sock_diag implementation as opt-in

- [ ] Add a feature flag `sock_diag` to `lazyadmin-adapter-procfs` (preferred to keeping discovery unified) or split into a sibling crate `lazyadmin-adapter-sockdiag`. Decision in PLAN-11 Phase 1 doc.
- [ ] Implement TCPv4/TCPv6/UDPv4/UDPv6 listener enumeration via sock_diag.
- [ ] Implement Unix-socket listener enumeration only if cheap; otherwise document gap and keep `/proc/net/unix` as the source.
- [ ] Map sock_diag results into the same `Listener` structs the procfs adapter produces; provenance kind = `SockDiag`.
- [ ] Orchestrator merge logic:
  - [ ] `preferred = "proc"` (default): no sock_diag work; behavior identical to v0.1.
  - [ ] `preferred = "sock_diag"`: sock_diag is primary; on any error, log `SOCK_DIAG_DOWNGRADED` warning and fall back to proc.
  - [ ] `preferred = "both"`: run both, treat sock_diag as primary, attach proc as corroborating provenance, raise warning on diff.
- [ ] Wire doctor `events`/`adapters.sockets` block to surface chosen path, runtime fallback, and any parity diff counts.

Tests:

- [ ] preferred=proc: behavior unchanged; sock_diag code path unreachable.
- [ ] preferred=sock_diag with fixture: produces same listener set, provenance shows `SockDiag`.
- [ ] preferred=sock_diag with simulated failure: orchestrator falls back to proc, doctor reports `SOCK_DIAG_DOWNGRADED`.
- [ ] preferred=both with intentional fixture diff: warning emitted; both provenance entries preserved.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs sockdiag
cargo run -p lazyadmin-cli -- doctor --json | jq '.subsystems.adapters.sockets'
```

## Phase 3 — Exact IPv6 dual-stack detection

Implements the v0.1 carry-over from PLAN-00.

- [ ] Add helper `probe_v6_only(pid: i32, fd: u32) -> Result<bool, ProbeError>`:
  - [ ] Open `/proc/<pid>/fd/<n>` with `O_PATH` to obtain a kernel-side FD reference. If kernel/permissions disallow, fall through.
  - [ ] Use `getsockopt(IPPROTO_IPV6, IPV6_V6ONLY)` on the resulting FD via `nix::sys::socket::getsockopt`.
  - [ ] Return distinct error variants for `PermissionDenied`, `NotAvailable`, `KernelRejected`, so the caller can decide between `unknown` and `possible`.
- [ ] Integrate into `lazyadmin-adapter-procfs`:
  - [ ] After socket-inode → PID/FD resolution (already in v0.1), if listener is IPv6 wildcard `[::]`, attempt `probe_v6_only` for each owning FD.
  - [ ] On success, set `dual_stack_state` to `confirmed_dual_stack` (V6ONLY=false) or `confirmed_v6_only` (V6ONLY=true).
  - [ ] On failure, leave `dual_stack_state = possible` and keep the v0.1 `possible_dual_stack` warning.
  - [ ] Hide warning when state is `confirmed_v6_only` (no dual-stack risk) or surface only the `[::]` warning for `confirmed_dual_stack`.
- [ ] Add IPv6 `[::]` listener to integration tests, including a test where probing fails (simulated permission error fixture).
- [ ] Update `docs/adapter-protocol.md` to note the new field and provenance.

Tests:

- [ ] dual-stack confirmed listener: state `confirmed_dual_stack`, warnings present but tagged confirmed.
- [ ] v6-only listener: state `confirmed_v6_only`, dual-stack warning suppressed.
- [ ] probe failure: state `possible`, warning unchanged from v0.1.
- [ ] non-IPv6 listener: state `not_applicable`.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs dualstack
```

## Phase 4 — DiscoveryEvent core wiring

- [ ] Implement `DiscoveryEvent` as in cross-cutting tasks; add small in-memory event normalizer in `lazyadmin-core/src/correlate/`:
  - [ ] Map raw adapter events to entity-level `Added/Removed/Changed`.
  - [ ] De-duplicate event storms within a configurable debounce window (default 250ms).
- [ ] Implement bounded fan-in:
  - [ ] Each adapter pushes events to its own task-local sender.
  - [ ] Orchestrator merges into a single `mpsc::channel(channel_capacity)`.
  - [ ] On overflow, drop oldest, increment `events_dropped` counter, emit `EVENTS_DROPPED` warning at next snapshot.
- [ ] Provide a CLI surface for events:
  - [ ] `lazyadmin events --json` streams normalized events as JSON Lines until interrupted.
  - [ ] `--once` flag prints the first event and exits, used in tests.
- [ ] Add fixture-driven tests using a fake adapter that emits scripted events.

Tests:

- [ ] event ordering preserved within an adapter.
- [ ] cross-adapter merge does not block when one adapter is slow.
- [ ] overflow drops oldest and reports `events_dropped`.
- [ ] `lazyadmin events --once` exits cleanly with valid JSON.

Validation:

```bash
cargo test -p lazyadmin-core discovery_event
cargo run -p lazyadmin-cli -- events --once --json | jq .
```

## Phase 5 — procfs `watch()` via debounced poll

The procfs adapter cannot subscribe to kernel events for `/proc/net` reliably. Use a debounced poll loop and emit only deltas.

- [ ] Implement `watch()` returning `Some(BoxStream<DiscoveryEvent>)`:
  - [ ] Run a tokio interval at `Config.ui.refresh_interval` (or a dedicated `adapters.events.procfs_interval`, default 1s).
  - [ ] Compare new scan to previous scan; emit `Added/Removed/Changed` events.
  - [ ] Coalesce events fired within the debounce window.
- [ ] Make polling cancellable cleanly (`tokio::select!` on shutdown signal).
- [ ] Emit `Heartbeat` events at most once every 5s when no real changes occurred, so the TUI can prove the source is alive.

Tests:

- [ ] starting/stopping a fixture process triggers `Added`/`Removed` events.
- [ ] no-change windows produce only `Heartbeat`.
- [ ] orchestrator can run watch + poll-based snapshots concurrently without double-counting.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs watch_loop
```

## Phase 6 — container `watch()` via Docker `/events`

- [ ] Wire `bollard::system::events()` into `lazyadmin-adapter-container`:
  - [ ] Filter to `type=container`, `type=network`, `type=volume` as needed; v0.2 needs container only.
  - [ ] Map raw Docker events (`start`, `stop`, `die`, `restart`, `kill`, `update`) to `DiscoveryEvent`.
  - [ ] Trigger an inspect refresh for the affected container ID and emit a normalized `Changed` event with field-level diffs (state, restart policy, published ports).
- [ ] Implement reconnection policy:
  - [ ] Exponential backoff up to 30s.
  - [ ] On reconnect, immediately request a list-containers refresh to avoid missed deltas.
- [ ] Document the policy in `docs/discovery-events-decision.md`.
- [ ] Podman: only enable events if the Podman socket implements Docker-compatible `/events`; otherwise skip with explicit health note. No new mutating Podman work in v0.2.

Tests:

- [ ] fixture-driven event stream through bollard mock; verify mapping.
- [ ] integration test (ignored unless Docker available) starting and stopping a small busybox container.

Validation:

```bash
cargo test -p lazyadmin-adapter-container events
```

## Phase 7 — systemd `watch()` via D-Bus PropertiesChanged

- [ ] Wire `zbus` `PropertiesChanged` signal subscription on `org.freedesktop.systemd1` for:
  - [ ] units already represented in the graph (filter by unit path/name set).
  - [ ] manager-level `JobNew`/`JobRemoved` for activity hints.
- [ ] On signal, re-fetch unit properties and emit `Changed` events; on `JobRemoved` with `unit not found`, emit `Removed`.
- [ ] Cap refetch concurrency to avoid dogpile.
- [ ] Health: report number of subscribed units, missed signals (if `zbus` exposes that), and last-event timestamp.
- [ ] Allow disabling via `adapters.systemd.events_enabled` (default `true`).

Tests:

- [ ] mocked signal triggers re-fetch and `Changed` event.
- [ ] disabled config -> watch returns `None` and procfs cgroup correlation still works in poll mode.
- [ ] integration test (ignored) with a user systemd target unit.

Validation:

```bash
cargo test -p lazyadmin-adapter-systemd events
```

## Phase 8 — Doctor and CLI surfacing

- [ ] Doctor JSON additions (still under `lazyadmin.doctor.v1` — additive fields):
  - [ ] `subsystems.adapters.sockets.preferred`
  - [ ] `subsystems.adapters.sockets.parity_diff_count`
  - [ ] `subsystems.adapters.sockets.dual_stack_probe.{ supported, attempted, succeeded, errors }`
  - [ ] `subsystems.events.{ enabled, per_adapter[].state, last_event_at, dropped }`
- [ ] Human doctor renders new sections with severity-colored badges.
- [ ] Add `lazyadmin events --follow` (already implied; ensure friendly output for humans without `--json`).
- [ ] Update `lazyadmin export --json`:
  - [ ] include `dual_stack_state` on listeners,
  - [ ] include `events_dropped` in metadata if non-zero.
- [ ] Update agent skill: add a small note in `skills/lazyadmin-agent/json-schema-v1.md` describing the new optional fields. Do **not** require agents to use them; backward compatibility holds.

Validation:

```bash
cargo run -p lazyadmin-cli -- doctor --json | jq '.subsystems.events'
cargo run -p lazyadmin-cli -- export --json | jq '.listeners[0].dual_stack_state // null'
```

## Phase 9 — Integration tests and CI hooks

- [ ] Add a Linux integration test `discovery_events_smoke` (gated, ignored by default):
  - [ ] starts a Python http.server, asserts an `Added` event arrives within 2s,
  - [ ] kills the process, asserts a `Removed` event within 2s,
  - [ ] cleans up cleanly.
- [ ] Wire the test under the existing CI integration job (manual dispatch / label) without making it a default gate.
- [ ] Add a fixture-only fast test for sock_diag parity that runs on every PR.

Validation:

```bash
cargo test --workspace
cargo test --workspace -- --ignored discovery_events_smoke
```

## Done criteria

- [ ] `Listener.dual_stack_state` is populated honestly: `confirmed_*` only when proven, `possible` otherwise.
- [ ] sock_diag adapter exists, opt-in, with parity tests passing against fixtures, and a documented decision record.
- [ ] `watch()` is implemented for procfs, container, and systemd adapters, with shutdown, reconnection, and bounded fan-in.
- [ ] `lazyadmin events --json` streams `lazyadmin.discovery_event.v1` payloads.
- [ ] Doctor reports new subsystems and degraded states.
- [ ] No JSON contract regressions; old consumers continue to work without changes.
- [ ] All `cargo fmt`/`clippy`/`test` workspace gates pass.

## Handoff notes for next plan

PLAN-12 consumes the new `DiscoveryEvent` channel directly: the TUI snapshot controller subscribes to events and redraws on event arrival or tick, whichever is first. PLAN-12 does not need to touch any adapter code. If event arrival proves bursty in real environments, PLAN-12 may add a per-view debounce on top of the channel; do not push that debounce back into adapters.
