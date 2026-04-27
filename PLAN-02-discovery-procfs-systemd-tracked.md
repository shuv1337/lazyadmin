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

- [ ] Add dependencies:
  - [ ] `procfs`
  - [ ] `nix`
  - [ ] `tokio`
  - [ ] `tracing`
  - [ ] `thiserror`
- [ ] Implement adapter config input from `Config.adapters.sockets` and `Config.ports`.
- [ ] Define internal structs:
  - [ ] `RawProcListener`
  - [ ] `RawProcProcess`
  - [ ] `SocketInode(u64)`
  - [ ] `NamespaceId`
  - [ ] `ProcScanCache`
- [ ] Add fixture loading helpers for parser tests so unit tests do not require live `/proc` only.

Telemetry:

- [ ] Span `adapter.procfs.discover` with listener counts, process counts, fd_readlink counts, cache hits, permission errors, duration.

## Phase 2 — `/proc/net` listener parsing

- [ ] Parse `/proc/net/tcp` and `/proc/net/tcp6`.
  - [ ] Decode hex IPv4/IPv6 addresses.
  - [ ] Decode ports.
  - [ ] Include only listen state for TCP unless config later asks for established.
  - [ ] Capture socket inode.
- [ ] Parse `/proc/net/udp` and `/proc/net/udp6`.
  - [ ] Treat bound UDP sockets as listeners/bindings, not TCP-style `LISTEN`.
  - [ ] Preserve state field for provenance/debug output.
- [ ] Parse `/proc/net/unix` when enabled.
  - [ ] Capture path, inode, socket type/state where available.
  - [ ] Hide Unix sockets from Everything view later, but include in JSON.
- [ ] Implement exposure classification:
  - [ ] loopback: `127.0.0.0/8`, `::1`, Unix socket,
  - [ ] LAN/public candidate: `0.0.0.0`, `::`, private LAN IPs, non-loopback interface IPs,
  - [ ] public: public-routable specific IP,
  - [ ] unknown when namespace/address is incomplete.
- [ ] Add warning caveat for `0.0.0.0`/`::`: "reachable beyond localhost depending on firewall/routing".
- [ ] Add dual-stack behavior as **best-effort** only:
  - [ ] For `[::]` listeners, add `possible_dual_stack` warning unless exact `IPV6_V6ONLY` proof is implemented.
  - [ ] Do not claim exact dual-stack if only `/proc/net` evidence exists.

Fallbacks:

- [ ] If parser fails, shell out to `ss` as a marked fallback adapter evidence source.
- [ ] Do not implement `lsof` fallback until a real test case requires it; leave trait seam.

Tests:

- [ ] IPv4 listen parser.
- [ ] IPv6 listen parser.
- [ ] UDP bound socket parser.
- [ ] Unix socket parser.
- [ ] malformed line resilience.
- [ ] exposure classification.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs proc_net
```

## Phase 3 — Process scan and cache

- [ ] Read boot ID from `/proc/sys/kernel/random/boot_id`.
- [ ] Walk numeric `/proc/<pid>` directories.
- [ ] For each process, collect:
  - [ ] PID,
  - [ ] start time ticks from `/proc/<pid>/stat`,
  - [ ] ppid, pgid, sid,
  - [ ] uid/gid from status,
  - [ ] exe symlink,
  - [ ] cwd symlink,
  - [ ] cmdline,
  - [ ] cgroup,
  - [ ] net namespace symlink/inode,
  - [ ] optional redacted environment summary only when enabled.
- [ ] Build `ProcessKey { pid, boot_id, start_time_ticks }` immediately after stat read.
- [ ] Cache stable metadata by `ProcessKey`.
- [ ] Distinguish permission errors:
  - [ ] `PermissionDenied` details become warnings/provenance, not hidden rows.
  - [ ] include PID/UID if known.
- [ ] Mark stale/deleted executable paths when symlink target includes `(deleted)`.
- [ ] Compute orphan marker when session leader has exited or reparented to PID 1 and no project/manager association exists later.

Tests:

- [ ] ProcessKey construction.
- [ ] cache hit/miss behavior.
- [ ] permission-denied fixture.
- [ ] deleted executable fixture.
- [ ] cmdline redaction integration.

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

- [ ] Implement inode set gating.
- [ ] Parse `socket:[123456]` symlink targets.
- [ ] Map multiple processes to same inode for `SO_REUSEPORT`/forked listeners.
- [ ] Do not assume one port equals one process.
- [ ] Add per-process FD read timeout/rate-limit guard.
- [ ] Track fd-read failures as warnings.

Tests:

- [ ] direct TCP listener fixture.
- [ ] reuseport/multiple PID fixture.
- [ ] PID reuse validation fixture.
- [ ] permission denied does not hide listener.

Validation:

```bash
cargo test -p lazyadmin-adapter-procfs socket_owner
```

## Phase 5 — Point query CLI for direct listeners

Crates: `lazyadmin-core`, `lazyadmin-cli`, `lazyadmin-adapter-procfs`.

- [ ] Wire procfs adapter into snapshot orchestrator.
- [ ] Implement `lazyadmin :PORT`, `lazyadmin port PORT`, and `--brief` for direct process listeners.
- [ ] Human output includes:
  - [ ] listener identity,
  - [ ] owner process summary,
  - [ ] cwd/project placeholder if unknown,
  - [ ] confidence,
  - [ ] warnings,
  - [ ] provenance.
- [ ] JSON output uses the same snapshot/query model, not a separate ad hoc struct.

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

- [ ] Add dependencies:
  - [ ] `zbus`
  - [ ] `tokio`
  - [ ] `tracing`
  - [ ] `thiserror`
- [ ] Implement health checks:
  - [ ] system bus reachable,
  - [ ] user bus reachable,
  - [ ] method availability hints where cheap,
  - [ ] permission/polkit hints for system-bus actions.
- [ ] Implement `Manager` entities for system and user buses.
- [ ] Implement timeout/cancellation for all D-Bus calls.

Telemetry:

- [ ] Span `adapter.systemd.health` and `adapter.systemd.discover` with bus, unit counts, failures, duration.

## Phase 7 — systemd process/unit correlation

- [ ] Bulk fast path:
  - [ ] Parse `/proc/<pid>/cgroup` from processes provided by procfs adapter.
  - [ ] Extract candidate unit names from cgroup paths.
  - [ ] Create medium/high confidence provenance depending on path clarity.
- [ ] Targeted verification:
  - [ ] Implement `GetUnitByPIDFD` when available.
  - [ ] Fall back to `GetUnitByPID`.
  - [ ] Fall back to `GetUnitByControlGroup` for cgroup-derived lookups.
- [ ] Represent system and user units distinctly:
  - [ ] `RuntimeKind::SystemdSystem`
  - [ ] `RuntimeKind::SystemdUser`
- [ ] Populate `Workload` entries for units with process refs.
- [ ] Add `ManagerOwnsWorkload` and `WorkloadContainsProcess` edges.
- [ ] Ensure point queries bypass default visibility filters later.

Tests:

- [ ] cgroup parser for system service.
- [ ] cgroup parser for user service.
- [ ] escaped unit names.
- [ ] fallback behavior when D-Bus methods unavailable.

Validation:

```bash
cargo test -p lazyadmin-adapter-systemd cgroup unit_lookup
```

## Phase 8 — systemd socket units and restart policy

- [ ] List socket units and service units through D-Bus.
- [ ] Spike exact property access for socket listeners:
  - [ ] `ListenStream`, `ListenDatagram`, `ListenSequentialPacket`, `Accept`, `Service`, `BindIPv6Only`, `ReusePort`, `SocketUser`, `SocketGroup`.
  - [ ] If properties are unavailable/incomplete via zbus, mark `systemctl show` or unit-file parsing fallback in provenance.
- [ ] Create socket `Workload` entries even with no PID.
- [ ] Create `WorkloadActivatedBy` edge from service to socket or socket to service consistently; document direction in `docs/adapter-protocol.md`.
- [ ] Populate `RestartPolicy` from service `Restart=` property.
- [ ] Add `SOCKET-ACT` warning/badge when socket owns listener and service inactive.

Tests:

- [ ] fixture for active socket/inactive service.
- [ ] restart policy parser.
- [ ] socket-to-listener matching by address/protocol/port.

Validation:

```bash
cargo test -p lazyadmin-adapter-systemd socket_units restart_policy
```

## Phase 9 — Tracked-run registry model

Crate: `crates/lazyadmin-adapter-tracked` plus core model additions.

- [ ] Define `TrackedRun` fields:
  - [ ] `id`,
  - [ ] `tag`,
  - [ ] `cmd`,
  - [ ] `cwd`,
  - [ ] `env_hash`,
  - [ ] `started_at`,
  - [ ] `creator`,
  - [ ] `scope_or_unit_name`,
  - [ ] `state`,
  - [ ] `log_source`,
  - [ ] `spawn_method`,
  - [ ] redacted metadata only.
- [ ] Registry path: `$XDG_RUNTIME_DIR/lazyadmin/runs/<id>.json` mode 0700 parent dir.
- [ ] Implement load/list/reconcile:
  - [ ] mark entries `exited` when backing scope/unit/process no longer exists,
  - [ ] never silently delete entries,
  - [ ] `forget` removes registry entry only.
- [ ] Add `lazyadmin runs --json` and human list.

Tests:

- [ ] registry read/write permissions.
- [ ] malformed registry entry quarantine.
- [ ] exited entry reconciliation.
- [ ] tag collision handling with `<tag>-<short-id>`.

## Phase 10 — Tracked-run spawn spike

This phase must complete before public `lazyadmin run` behavior is documented.

Spike matrix:

- [ ] `systemd-run --user --scope --unit=lazyadmin-run-<id>.scope --collect -- <cmd>` foreground.
- [ ] Scope plus `lazyadmin` shim process that redirects stdout/stderr to file and can detach.
- [ ] Transient user service via `systemd-run --user --unit=lazyadmin-run-<id>.service --collect --property=Type=exec ...`.
- [ ] Direct D-Bus `StartTransientUnit` equivalent for both scope/service candidates if CLI shell-out proves too limiting.

Evaluate each candidate for:

- [ ] returns immediately for `--detach`,
- [ ] preserves desired cwd/env,
- [ ] captures descendant tree reliably,
- [ ] exposes cgroup/unit for discovery,
- [ ] provides logs through journal or file,
- [ ] stops descendants cleanly,
- [ ] works without lingering after logout expectations documented,
- [ ] handles command-not-found errors clearly.

Decision record:

- [ ] Write `docs/tracked-run-spawn-decision.md` with chosen behavior and rejected alternatives.
- [ ] Update `PLAN-02` checklist or open follow-up if behavior differs from original spec.

User-confirmed planning preference:

- Keep `systemd-run --user --scope` as the candidate, but explicitly validate/downgrade log claims. If the spike proves scopes cannot satisfy `--detach` and logs safely, implement a hybrid while preserving the same external JSON fields.

## Phase 11 — Tracked-run MVP implementation

Implement only after Phase 10 decision.

- [ ] `lazyadmin run --tag NAME --detach --cwd PATH --env KEY=VAL -- <cmd>`.
- [ ] `lazyadmin run stop <id|tag>`.
- [ ] `lazyadmin run logs <id|tag>` using proven `log_source`.
- [ ] `lazyadmin run forget <id|tag>`.
- [ ] `lazyadmin run restart <id|tag>` only if original command/env/cwd can be restored safely; otherwise defer with clear message.
- [ ] Join tracked registry with systemd/procfs discovery:
  - [ ] set `Process.lazyadmin_run_id`,
  - [ ] create `RuntimeKind::LazyadminTracked` workload,
  - [ ] add `TrackedRunSpawned` edge,
  - [ ] add `TRACKED` badge/warning.
- [ ] Update direct-process no-log message to suggest `lazyadmin run`.

Tests:

- [ ] tracked run appears in `runs --json`.
- [ ] tracked run appears in `export --json` workloads and tracked_runs.
- [ ] stop terminates child and grandchild.
- [ ] logs command reflects `log_source` truthfully.
- [ ] registry survives CLI restart during same boot.

Validation:

```bash
cargo run -p lazyadmin-cli -- run --tag la-smoke --detach -- python3 -m http.server 3010
cargo run -p lazyadmin-cli -- runs --json | jq '.tracked_runs[] | select(.tag=="la-smoke")'
cargo run -p lazyadmin-cli -- :3010 --json | jq .
cargo run -p lazyadmin-cli -- run stop tag:la-smoke
```

## Phase 12 — Adapter health and doctor inputs

Expose structured health records for `PLAN-04` doctor rendering.

- [ ] procfs health:
  - [ ] `/proc` readable,
  - [ ] `/proc/net` readable,
  - [ ] fallback `ss` availability,
  - [ ] unreadable process count.
- [ ] systemd health:
  - [ ] user/system bus reachability,
  - [ ] journal availability placeholder,
  - [ ] method availability warnings,
  - [ ] polkit/sudo requirement hints.
- [ ] tracked health:
  - [ ] `$XDG_RUNTIME_DIR` present,
  - [ ] registry dir writable,
  - [ ] spawn method decision/availability,
  - [ ] active/exited run counts.

## Done criteria

- [ ] Direct TCP/UDP/Unix listeners are discoverable from `/proc/net`.
- [ ] Listener inode to owning process mapping works with provenance.
- [ ] Permission-denied processes are visible as partial data.
- [ ] `lazyadmin :PORT --brief` works for a direct listener.
- [ ] systemd system/user units correlate to processes through cgroups and targeted D-Bus verification.
- [ ] systemd socket activation can be represented without a service PID.
- [ ] Restart policy is populated for systemd service workloads.
- [ ] Tracked-run spawn behavior has a documented decision record.
- [ ] `lazyadmin run --detach`, `runs --json`, `run stop`, and `run logs` are implemented only to the level proven by the spike.
- [ ] All discovery phases emit structured tracing spans.

## Handoff notes for next plan

`PLAN-03` should treat procfs/systemd/tracked outputs as separate evidence sources and perform graph-level correlation in core. Do not make the container adapter depend on systemd internals except through shared model fields like cgroups and process/container IDs.
