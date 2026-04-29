# PLAN-02 — Procfs/Sockets, systemd, and Tracked Runtime Discovery

Source: `lazyadmin-spec-v0_2.md` sections 10.1–10.3, 10.7, 11, 15, 19–20, 23–25, 27–28, 32.  
Depends on: `PLAN-01-foundation-core-cli-json.md`.  
Goal: implement the Linux discovery backbone: `/proc/net` listeners, process enrichment, socket-inode ownership, systemd unit/socket correlation, and tracked-run registry/spawn behavior after proving the `systemd-run` assumptions.

## Key risks to resolve first

- `systemd-run --user --scope` is synchronous and may not provide journal-backed stdout/stderr history by itself.
- `/proc/net` does not prove all per-socket options, especially exact IPv6 `IPV6_V6ONLY`.
- systemd D-Bus API availability varies by systemd version.
- Process scans race with PID reuse; every persisted process relation must use `ProcessKey`.

## Phase 1 — Procfs/sockets adapter crate setup

Crate: `crates/lazyadmin-adapter-procfs`.

- [x] Add dependencies:
  - [x] `procfs`
  - [x] `nix`
  - [x] `tokio`
  - [x] `tracing`
  - [x] `thiserror`
- [x] Implement adapter config input from `Config.adapters.sockets` and `Config.ports`.
- [x] Define internal structs:
  - [x] `RawProcListener`
  - [x] `RawProcProcess`
  - [x] `SocketInode(u64)`
  - [x] `NamespaceId`
  - [x] `ProcScanCache`
- [x] Add fixture loading helpers for parser tests so unit tests do not require live `/proc` only.

Telemetry:

- [x] Span `adapter.procfs.discover` with listener counts, process counts, fd_readlink counts, cache hits, permission errors, duration.

## Phase 2 — `/proc/net` listener parsing

- [x] Parse `/proc/net/tcp` and `/proc/net/tcp6`.
  - [x] Decode hex IPv4/IPv6 addresses.
  - [x] Decode ports.
  - [x] Include only listen state for TCP unless config later asks for established.
  - [x] Capture socket inode.
- [x] Parse `/proc/net/udp` and `/proc/net/udp6`.
  - [x] Treat bound UDP sockets as listeners/bindings, not TCP-style `LISTEN`.
  - [x] Preserve state field for provenance/debug output.
- [x] Parse `/proc/net/unix` when enabled.
  - [x] Capture path, inode, socket type/state where available.
  - [x] Hide Unix sockets from Everything view later, but include in JSON.
- [x] Implement exposure classification:
  - [x] loopback: `127.0.0.0/8`, `::1`, Unix socket,
  - [x] LAN/public candidate: `0.0.0.0`, `::`, private LAN IPs, non-loopback interface IPs,
  - [x] public: public-routable specific IP,
  - [x] unknown when namespace/address is incomplete.
- [x] Add warning caveat for `0.0.0.0`/`::`: "reachable beyond localhost depending on firewall/routing".
- [x] Add dual-stack behavior as **best-effort** only:
  - [x] For `[::]` listeners, add `possible_dual_stack` warning unless exact `IPV6_V6ONLY` proof is implemented.
  - [x] Do not claim exact dual-stack if only `/proc/net` evidence exists.

Fallbacks:

- [x] If parser fails, shell out to `ss` as a marked fallback adapter evidence source.
- [x] Do not implement `lsof` fallback until a real test case requires it; leave trait seam.

Tests:

- [x] IPv4 listen parser.
- [x] IPv6 listen parser.
- [x] UDP bound socket parser.
- [x] Unix socket parser.
- [x] malformed line resilience.
- [x] exposure classification.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs proc_net
```

## Phase 3 — Process scan and cache

- [x] Read boot ID from `/proc/sys/kernel/random/boot_id`.
- [x] Walk numeric `/proc/<pid>` directories.
- [x] For each process, collect:
  - [x] PID,
  - [x] start time ticks from `/proc/<pid>/stat`,
  - [x] ppid, pgid, sid,
  - [x] uid/gid from status,
  - [x] exe symlink,
  - [x] cwd symlink,
  - [x] cmdline,
  - [x] cgroup,
  - [x] net namespace symlink/inode,
  - [x] optional redacted environment summary only when enabled.
- [x] Build `ProcessKey { pid, boot_id, start_time_ticks }` immediately after stat read.
- [x] Cache stable metadata by `ProcessKey`.
- [x] Distinguish permission errors:
  - [x] `PermissionDenied` details become warnings/provenance, not hidden rows.
  - [x] include PID/UID if known.
- [x] Mark stale/deleted executable paths when symlink target includes `(deleted)`.
- [x] Compute orphan marker when session leader has exited or reparented to PID 1 and no project/manager association exists later.

Tests:

- [x] ProcessKey construction.
- [x] cache hit/miss behavior.
- [x] permission-denied fixture.
- [x] deleted executable fixture.
- [x] cmdline redaction integration.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs process_scan
```

## Phase 4 — Socket inode to PID mapping

Algorithm:

1. Build listener inode set from parsed listeners.
2. Walk process table.
3. For uncached or unowned processes, read `/proc/<pid>/fd/*` symlinks.
4. Stop early for a process once all relevant listener inodes it owns are found.
5. Validate process `start_time_ticks` did not change between scan and FD walk.
6. Emit `ProcessOwnsListener` edges with high-confidence provenance.

Tasks:

- [x] Implement inode set gating.
- [x] Parse `socket:[123456]` symlink targets.
- [x] Map multiple processes to same inode for `SO_REUSEPORT`/forked listeners.
- [x] Do not assume one port equals one process.
- [x] Add per-process FD read timeout/rate-limit guard.
- [x] Track fd-read failures as warnings.

Tests:

- [x] direct TCP listener fixture.
- [x] reuseport/multiple PID fixture.
- [x] PID reuse validation fixture.
- [x] permission denied does not hide listener.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs socket_owner
```

## Phase 5 — Point query CLI for direct listeners

Crates: `lazyadmin-core`, `lazyadmin-cli`, `lazyadmin-adapter-procfs`.

- [x] Wire procfs adapter into snapshot orchestrator.
- [x] Implement `lazyadmin :PORT`, `lazyadmin port PORT`, and `--brief` for direct process listeners.
- [x] Human output includes:
  - [x] listener identity,
  - [x] owner process summary,
  - [x] cwd/project placeholder if unknown,
  - [x] confidence,
  - [x] warnings,
  - [x] provenance.
- [x] JSON output uses the same snapshot/query model, not a separate ad hoc struct.

Validation:

```bash
# manual smoke
python3 -m http.server 3000 >/tmp/lazyadmin-http.log 2>&1 & echo $! > /tmp/lazyadmin-http.pid
cargo run -p lazyadmin-cli -- :3000 --brief
cargo run -p lazyadmin-cli -- :3000 --json | jq .
kill $(cat /tmp/lazyadmin-http.pid)
```

## Phase 6 — systemd adapter crate setup

Crate: `crates/lazyadmin-adapter-systemd`.

- [x] Add dependencies:
  - [x] `zbus`
  - [x] `tokio`
  - [x] `tracing`
  - [x] `thiserror`
- [x] Implement health checks:
  - [x] system bus reachable,
  - [x] user bus reachable,
  - [x] method availability hints where cheap,
  - [x] permission/polkit hints for system-bus actions.
- [x] Implement `Manager` entities for system and user buses.
- [x] Implement timeout/cancellation for all D-Bus calls.

Telemetry:

- [x] Span `adapter.systemd.health` and `adapter.systemd.discover` with bus, unit counts, failures, duration.

## Phase 7 — systemd process/unit correlation

- [x] Bulk fast path:
  - [x] Parse `/proc/<pid>/cgroup` from processes provided by procfs adapter.
  - [x] Extract candidate unit names from cgroup paths.
  - [x] Create medium/high confidence provenance depending on path clarity.
- [x] Targeted verification:
  - [x] Implement `GetUnitByPIDFD` when available.
  - [x] Fall back to `GetUnitByPID`.
  - [x] Fall back to `GetUnitByControlGroup` for cgroup-derived lookups.
- [x] Represent system and user units distinctly:
  - [x] `RuntimeKind::SystemdSystem`
  - [x] `RuntimeKind::SystemdUser`
- [x] Populate `Workload` entries for units with process refs.
- [x] Add `ManagerOwnsWorkload` and `WorkloadContainsProcess` edges.
- [x] Ensure point queries bypass default visibility filters later.

Tests:

- [x] cgroup parser for system service.
- [x] cgroup parser for user service.
- [x] escaped unit names.
- [x] fallback behavior when D-Bus methods unavailable.

Validation:

```bash
cargo test -p lazyadmin-adapter-systemd cgroup unit_lookup
```

## Phase 8 — systemd socket units and restart policy

- [x] List socket units and service units through D-Bus.
- [x] Spike exact property access for socket listeners:
  - [x] `ListenStream`, `ListenDatagram`, `ListenSequentialPacket`, `Accept`, `Service`, `BindIPv6Only`, `ReusePort`, `SocketUser`, `SocketGroup`.
  - [x] If properties are unavailable/incomplete via zbus, mark `systemctl show` or unit-file parsing fallback in provenance.
- [x] Create socket `Workload` entries even with no PID.
- [x] Create `WorkloadActivatedBy` edge from service to socket or socket to service consistently; document direction in `docs/adapter-protocol.md`.
- [x] Populate `RestartPolicy` from service `Restart=` property.
- [x] Add `SOCKET-ACT` warning/badge when socket owns listener and service inactive.

Tests:

- [x] fixture for active socket/inactive service.
- [x] restart policy parser.
- [x] socket-to-listener matching by address/protocol/port.

Validation:

```bash
cargo test -p lazyadmin-adapter-systemd socket_units restart_policy
```

## Phase 9 — Tracked-run registry model

Crate: `crates/lazyadmin-adapter-tracked` plus core model additions.

- [x] Define `TrackedRun` fields:
  - [x] `id`,
  - [x] `tag`,
  - [x] `cmd`,
  - [x] `cwd`,
  - [x] `env_hash`,
  - [x] `started_at`,
  - [x] `creator`,
  - [x] `scope_or_unit_name`,
  - [x] `state`,
  - [x] `log_source`,
  - [x] `spawn_method`,
  - [x] redacted metadata only.
- [x] Registry path: `$XDG_RUNTIME_DIR/lazyadmin/runs/<id>.json` mode 0700 parent dir.
- [x] Implement load/list/reconcile:
  - [x] mark entries `exited` when backing scope/unit/process no longer exists,
  - [x] never silently delete entries,
  - [x] `forget` removes registry entry only.
- [x] Add `lazyadmin runs --json` and human list.

Tests:

- [x] registry read/write permissions.
- [x] malformed registry entry quarantine.
- [x] exited entry reconciliation.
- [x] tag collision handling with `<tag>-<short-id>`.

## Phase 10 — Tracked-run spawn spike

This phase must complete before public `lazyadmin run` behavior is documented.

Spike matrix:

- [x] `systemd-run --user --scope --unit=lazyadmin-run-<id>.scope --collect -- <cmd>` foreground.
- [x] Scope plus `lazyadmin` shim process that redirects stdout/stderr to file and can detach.
- [x] Transient user service via `systemd-run --user --unit=lazyadmin-run-<id>.service --collect --property=Type=exec ...`.
- [x] Direct D-Bus `StartTransientUnit` equivalent for both scope/service candidates if CLI shell-out proves too limiting.

Evaluate each candidate for:

- [x] returns immediately for `--detach`,
- [x] preserves desired cwd/env,
- [x] captures descendant tree reliably,
- [x] exposes cgroup/unit for discovery,
- [x] provides logs through journal or file,
- [x] stops descendants cleanly,
- [x] works without lingering after logout expectations documented,
- [x] handles command-not-found errors clearly.

Decision record:

- [x] Write `docs/tracked-run-spawn-decision.md` with chosen behavior and rejected alternatives.
- [x] Update `PLAN-02` checklist or open follow-up if behavior differs from original spec.

User-confirmed planning preference:

- Keep `systemd-run --user --scope` as the candidate, but explicitly validate/downgrade log claims. If the spike proves scopes cannot satisfy `--detach` and logs safely, implement a hybrid while preserving the same external JSON fields.

## Phase 11 — Tracked-run MVP implementation

Implement only after Phase 10 decision.

- [x] `lazyadmin run --tag NAME --detach --cwd PATH --env KEY=VAL -- <cmd>`.
- [x] `lazyadmin run stop <id|tag>`.
- [x] `lazyadmin run logs <id|tag>` using proven `log_source`.
- [x] `lazyadmin run forget <id|tag>`.
- [x] `lazyadmin run restart <id|tag>` only if original command/env/cwd can be restored safely; otherwise defer with clear message.
- [x] Join tracked registry with systemd/procfs discovery:
  - [x] set `Process.lazyadmin_run_id`,
  - [x] create `RuntimeKind::LazyadminTracked` workload,
  - [x] add `TrackedRunSpawned` edge,
  - [x] add `TRACKED` badge/warning.
- [x] Update direct-process no-log message to suggest `lazyadmin run`.

Tests:

- [x] tracked run appears in `runs --json`.
- [x] tracked run appears in `export --json` workloads and tracked_runs.
- [x] stop terminates child and grandchild.
- [x] logs command reflects `log_source` truthfully.
- [x] registry survives CLI restart during same boot.

Validation:

```bash
cargo run -p lazyadmin-cli -- run --tag la-smoke --detach -- python3 -m http.server 3010
cargo run -p lazyadmin-cli -- runs --json | jq '.tracked_runs[] | select(.tag=="la-smoke")'
cargo run -p lazyadmin-cli -- :3010 --json | jq .
cargo run -p lazyadmin-cli -- run stop tag:la-smoke
```

## Phase 12 — Adapter health and doctor inputs

Expose structured health records for `PLAN-04` doctor rendering.

- [x] procfs health:
  - [x] `/proc` readable,
  - [x] `/proc/net` readable,
  - [x] fallback `ss` availability,
  - [x] unreadable process count.
- [x] systemd health:
  - [x] user/system bus reachability,
  - [x] journal availability placeholder,
  - [x] method availability warnings,
  - [x] polkit/sudo requirement hints.
- [x] tracked health:
  - [x] `$XDG_RUNTIME_DIR` present,
  - [x] registry dir writable,
  - [x] spawn method decision/availability,
  - [x] active/exited run counts.

## Done criteria

- [x] Direct TCP/UDP/Unix listeners are discoverable from `/proc/net`.
- [x] Listener inode to owning process mapping works with provenance.
- [x] Permission-denied processes are visible as partial data.
- [x] `lazyadmin :PORT --brief` works for a direct listener.
- [x] systemd system/user units correlate to processes through cgroups and targeted D-Bus verification.
- [x] systemd socket activation can be represented without a service PID.
- [x] Restart policy is populated for systemd service workloads.
- [x] Tracked-run spawn behavior has a documented decision record.
- [x] `lazyadmin run --detach`, `runs --json`, `run stop`, and `run logs` are implemented only to the level proven by the spike.
- [x] All discovery phases emit structured tracing spans.

## Handoff notes for next plan

`PLAN-03` should treat procfs/systemd/tracked outputs as separate evidence sources and perform graph-level correlation in core. Do not make the container adapter depend on systemd internals except through shared model fields like cgroups and process/container IDs.
