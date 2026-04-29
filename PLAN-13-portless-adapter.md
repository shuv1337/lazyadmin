# PLAN-13 — portless adapter (read-only manager + manager-aware `free`)

Source: this plan; aligned with `lazyadmin-spec-v0_2.md` §10 (discovery adapters), §11 (correlation), §14.3 (free-port algorithm), §23 (adapter trait design).
Depends on: `PLAN-10..12` (v0.2 baseline shipped).
Status: implemented; real-portless dogfood passed; released as v0.3.0.

## What changed in this revision

The previous draft was based on second-hand assumptions about `vercel-labs/portless` and on a misreading of `lazyadmin-core`'s shape. Verifying both moved several decisions:

- **`portless prune` does not take `--hostname`.** It only accepts `--force` and `--help`, and only ever cleans up routes whose owning *CLI* process is dead. It is an orphan reaper, not a per-route stop command. (`packages/portless/src/cli.ts` `handlePrune`)
- **`route.pid` is the portless CLI process, not the dev-server.** `runApp` calls `store.addRoute(hostname, port, process.pid, force)`. The dev-server is a *descendant* of `spawnCommand`'s `/bin/sh -c <cmd>` shell, started with `detached: true` so `killTree(-pid)` reaches the whole subtree. lazyadmin's listener-PID equality with `route.pid` will essentially never match in practice.
- **The lock is a `mkdir`-style directory** at `routes.lock`, with a 10s stale threshold and a 5s acquire budget. There is no `flock` involved. (`packages/portless/src/routes.ts` `acquireLock`)
- **The state dir is `~/.portless`** (since 0.11.0), with `/tmp/portless` as a read-only legacy fallback and `PORTLESS_STATE_DIR` as an override. (`packages/portless/src/cli-utils.ts` `USER_STATE_DIR`, `LEGACY_SYSTEM_STATE_DIR`, `resolveStateDir`). lazyadmin should read the explicit override when set; otherwise it should read `~/.portless` plus `/tmp/portless` when the legacy dir exists, with provenance identifying which dir produced each route.
- **The on-disk schema is exactly three fields**: `[{ hostname: string, port: number, pid: number }]`. **`pid: 0` is an alias** registered with `portless alias` (e.g. for Docker) — there is no live owner.
- **lazyadmin's `Action`/`ActionKind` lives in `lazyadmin_core::actions`, not `model`**. The plan needs to add the new variant in `actions/mod.rs`, not `model/mod.rs`.
- **Listener output rows do not carry a `manager` field today, and `ps --json` currently returns the raw `Snapshot`.** Surfacing portless labels in human/TUI rows requires adding a projection helper, but the plan must not promise `.listeners[].manager_label` in snapshot JSON unless it intentionally changes the public snapshot contract.
- **The portless CLI PID needs a structured home.** Do not leave it only in a provenance string. Phase 2 resolves `route.pid` into a `ProcessKey` and stores it in `Workload.source = Some(EntityRef::Process(cli_key))`; `Workload.pids` remains for listener-owning descendant process keys.
- **`PortlessPrune` is deferred.** Doctor can recommend `portless prune`, but this plan does not add a prune action or doctor remediation command until lazyadmin has a first-class remediation/action contract for doctor output.

The revision keeps the original goal — make lazyadmin aware of portless and route `free` through the right per-manager path — but replaces the wrong execution path with a correct one and shrinks the surface so each phase ships behind real evidence.

## Why

`portless` is a single-process DX wrapper that gives a dev server a stable `*.localhost` URL. Its state lives in `~/.portless/routes.json`, one entry per active app. lazyadmin already owns "which manager owns this listener" for procfs / systemd / containers / lazyadmin-tracked. Adding a `portless` adapter:

- closes a real gap on the user's box (multiple long-lived `portless myapp …` processes are common during dev),
- gives `free <port>` a manager-aware path so it stops the portless CLI cleanly (which in turn `killTree`s the dev-server child and removes the route via portless's own `onCleanup`), instead of issuing a bare SIGTERM that may race the cleanup, and
- is a clean fit for the existing adapter contract — additive `RuntimeKind` / `ActionKind` discriminants, no public snapshot JSON shape changes beyond those new variants.

This is the smallest credible piece of the larger "lazyadmin as nervous system, portless as a consumer" story. Ship this first; the bigger pieces (`portless run` delegating supervision to `lazyadmin run`, `lazyadmin.discovery_event.v1` subscription) are out of scope here and tracked under "Deferred" below.

## Non-goals

- No mutation of `~/.portless/routes.json` or `/tmp/portless/routes.json`. lazyadmin reads, portless writes. Cleanup happens by signaling the portless CLI process (which removes its own route on exit); orphan cleanup is reported as a doctor hint to run `portless prune`, never by editing JSON directly.
- No replacement of `portless`. Coexistence only.
- No schema-version bump for `lazyadmin.snapshot.v1`, `lazyadmin.diff.v1`, or `lazyadmin.action_report.v1`. New `RuntimeKind` and `ActionKind` enum variants are additive. Per `docs/versioning.md`, JSON contract evolution requires a schema bump only for *breaking* changes; additive enum variants follow precedent (every prior adapter added new variants without bumping). The CHANGELOG must still flag the new variants for strict-schema consumers.
- No new `manager_label` field in `Snapshot.listeners` or `ps --json` / `public --json`. Manager labels are row projections for human CLI output and TUI/headless TUI view models. JSON consumers can derive the same information from `edges` + `workloads`.
- No watching of `~/.portless/routes.json` in this plan. Snapshot polling is authoritative; live watch is a deferred follow-up.
- No `ActionKind::PortlessPrune` in this plan. `portless prune` remains an external remediation hint until doctor has a typed recommended-action surface.

## Scope

In:

- New crate `crates/lazyadmin-adapter-portless` implementing `DiscoveryAdapter` per `lazyadmin-core::graph`.
- One `Manager { kind: RuntimeKind::Portless, scope: ManagerScope::User }` per resolved readable state directory. Resolution rule: if `PORTLESS_STATE_DIR` is set, use that single override; otherwise inspect `~/.portless` and, if present, `/tmp/portless` as a read-only legacy state dir.
- One `Workload { runtime: RuntimeKind::Portless }` per **live** route (`pid != 0` and `/proc/<pid>` exists). Aliases (`pid == 0`) are recorded as `Workload`s with empty `pids` and a "static alias" provenance line — they get no `free` action.
- Edges: `ManagerOwnsWorkload` (portless manager → workload). `WorkloadOwnsListener` is **not** emitted by this adapter, because the portless CLI pid does not own the listener at `127.0.0.1:<port>`; ownership is established by post-correlation walking from `route.pid` to descendant pids that procfs claims own the listener (see Correlation below).
- New `ActionKind::PortlessStop` (subprocess: SIGTERM to the portless CLI pid). This relies on portless's documented signal handling — `cli-utils.ts` `spawnCommand.handleSignal` → `killTree(child, signal)` + `onCleanup` → `store.removeRoute(hostname)` — to take down the dev-server tree and self-remove its route.
- Inspector / human `ps` / human `public` row label: append `(portless: <hostname>.<tld>)` to the existing line; introduce an optional `manager_label` projection field on an output `ListenerRow` helper and on TUI row/view-model data. Do not add it to `Snapshot.listeners`.
- `doctor` checks: state dir resolvable, `routes.json` parseable, `routes.lock` not stale, count of orphan routes (alive port, dead CLI), proxy daemon process status (presence of `proxy.pid` and process liveness), and `portless` binary on PATH (info-severity if missing while routes exist).

Out (this plan):

- Live watch of `~/.portless/routes.json` (no `notify` integration yet).
- `portless run` delegating supervision to `lazyadmin run`.
- Subscribing portless to `lazyadmin.discovery_event.v1`.
- Windows / WSL parity (portless supports Windows; lazyadmin is Linux-first — out of scope).

## Public surface changes (additive)

`crates/lazyadmin-core/src/model/mod.rs`:

- `RuntimeKind::Portless` (new variant; `serde(rename_all = "snake_case")` keeps it as `"portless"`).

`crates/lazyadmin-core/src/actions/mod.rs`:

- `ActionKind::PortlessStop` (additive variant on the existing enum).

`crates/lazyadmin-core/src/output/mod.rs`:

- `ListenerRow` projection helper with existing listener row fields plus `manager_label: Option<String>` and `manager_detail: Option<String>`.
- This helper is used by human `ps` / `public` rendering and TUI/headless TUI view-model construction. It is **not** the public `Snapshot` shape; `ps --json`, `public --json`, `export --json`, and `diff --json` continue to serialize snapshots/diffs.

No changes to `Manager`, `Workload`, `Edge`, `EdgeKind`, `ManagerScope`, core `Listener`, or any schema id. Reuse `ManagerScope::User`. Use existing `Workload.source` to store the resolved portless CLI `ProcessKey` once correlation can prove it.

JSON contract impact: consumers of `lazyadmin.snapshot.v1`, `lazyadmin.diff.v1`, and `lazyadmin.action_report.v1` must tolerate the additive enum variants `runtime: "portless"` and `action.kind: "portless_stop"`. `manager_label` is intentionally absent from snapshot JSON; scripts should derive it by following `EdgeKind::WorkloadOwnsListener` to a `Workload { runtime: "portless" }`. Phase 1 verifies that `testdata/snapshots/` golden tests round-trip a snapshot containing `runtime: "portless"` without diff churn for unrelated fixtures.

## Adapter contract (new crate)

`crates/lazyadmin-adapter-portless/src/lib.rs`:

```rust
pub struct PortlessAdapter {
    /// Resolved at construction. If PORTLESS_STATE_DIR is set, this contains only
    /// that override. Otherwise it contains ~/.portless and, when present,
    /// /tmp/portless as a read-only legacy fallback.
    state_dirs: Vec<PathBuf>,
    /// Resolved once via `which portless`. None means manager.available = false
    /// and doctor reports binary availability as Info when routes exist.
    portless_bin: Option<PathBuf>,
}

#[async_trait]
impl DiscoveryAdapter for PortlessAdapter {
    fn name(&self) -> &'static str { "portless" }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities { polling: true, watching: false } // watch deferred
    }
    async fn health(&self) -> AdapterHealth { /* ... */ }
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> { /* ... */ }
}
```

`discover` reads each resolved `routes.json` once per snapshot pass:

1. Resolve state dirs at construction:
   - `PORTLESS_STATE_DIR` set → use only that path.
   - Otherwise use `$HOME/.portless`; also include `/tmp/portless` when that directory exists. The legacy path is read-only and never created by lazyadmin.
2. If no state dir exists → return empty `DiscoveryOutput` with `health.available = false` and message `"portless state directory not present"`. Do not emit a manager.
3. **Read without locking.** portless's lock is `mkdir routes.lock` with a 10s stale threshold; participating in the lock protocol from a read-only consumer adds risk and is unnecessary for our use case. Read each file with a single `tokio::fs::read`. If the JSON parse fails for one state dir, emit one `Warning { code: "portless.routes_unparseable", severity: Warning }` for that dir and continue with any remaining state dirs; portless will overwrite the file on its next mutation.
4. Validate each entry against the documented shape (`hostname: string, port: number, pid: number`). Drop entries that fail validation; emit one aggregated warning rather than per-entry.
5. For each valid entry:
   - `pid == 0` → **alias**. Emit `Workload { runtime: Portless, state: Running, pids: [], display_name: <hostname>, manager: Some(<the portless manager id>), provenance: "static alias" }`. No further action.
   - `pid != 0`, `/proc/<pid>` exists → **live**. Emit `Workload` with `pids: vec![]`, `source: None`, and a `Provenance` line carrying the route's bare `pid: i32` in `evidence` (e.g. `"routes.json pid=2728641 state_dir=/home/me/.portless"`). Resolving the pid into a full `ProcessKey { pid, boot_id, start_time_ticks }` and storing `source: Some(EntityRef::Process(cli_key))` is correlation's job, because procfs has the boot/start-time guard. Liveness check uses `Path::new("/proc").join(pid.to_string()).exists()` only as a coarse adapter filter.
   - `pid != 0`, `/proc/<pid>` missing → **orphan**. Emit `Warning { code: "portless.orphan_route", severity: Warning, message: "route '<host>' :<port> has dead CLI pid <pid>; run `portless prune` to clean up" }`. Do **not** emit a workload (the route is stale).
6. Emit a single `Manager` per existing state dir:
   - `id: ManagerId::new(format!("portless:{}", state_dir.display()))`,
   - `kind: RuntimeKind::Portless`,
   - `scope: ManagerScope::User`,
   - `socket: None`,
   - `available: portless_bin.is_some()` (the binary, not the daemon — daemon liveness is reported as a doctor check, not as manager availability),
   - `permission: PermissionState::Ok` when readable, `Denied` when `routes.json` returns EACCES, `Unknown` otherwise,
   - `version: <cached portless --version output>` if the binary is present (cached for the adapter's lifetime; do not re-shell per snapshot),
   - `provenance: ["state dir = <path>", "binary = <path|missing>", "legacy = true|false"]`.

### Correlation (in `lazyadmin-core::correlate`)

`route.pid` does not own the listener; it owns the `/bin/sh -c <cmd>` parent that `spawnCommand` started with `detached: true`. The dev-server lives somewhere in that subtree. The cheapest correct correlation is:

1. For each portless workload `w` with `route.pid = P`:
2. Resolve `P` to a procfs `ProcessKey` by finding the current `Process { pid: P }`. If there is no process, leave the workload uncorrelated and emit a `portless.route_pid_missing` warning. If there are multiple impossible candidates, refuse to correlate.
3. Store that exact key as `w.source = Some(EntityRef::Process(cli_key))`. This is the structured PID-reuse guard used later by `free`; do not make `free` parse a provenance string.
4. Build a parent→children index from `Graph.processes` using `Process.ppid`.
5. Walk the procfs `ppid` tree downward from the CLI pid: BFS through children whose `ppid` is in the set of known descendants, with a depth cap of 8.
6. Use `pgid == P` only as secondary evidence in provenance when present. Do not require the pgrp relationship for correctness; the parent-child tree is the authority.
7. For every listener whose owning `ProcessRef` is in that descendant set, emit `Edge::WorkloadOwnsListener` from `w` to that listener with confidence `Medium` (process tree shape can change between scans). Also make the output projection compute `manager_label = Some("portless")` from that edge.

Provenance string: `"procfs-descendant-of portless cli pid <P>"`.

If the procfs adapter does not already expose a parent→children helper, add one in `lazyadmin-core::correlate` over the already-merged `Graph.processes` (`fn children_index(processes: &IndexMap<ProcessKey, Process>) -> HashMap<i32, Vec<ProcessKey>>`). This is a Phase 2 task.

## `free <port>` integration

Today `run_free` plans `plan_direct_process` for every owning pid and signals SIGTERM. For portless, that is *almost* right — portless's signal handler does the right thing — but:

- the listener owner is the dev-server descendant, not the portless CLI, so naive `plan_direct_process` would SIGTERM the descendant. Portless's `onCleanup` only runs when the parent CLI exits, so the route would be left behind and only the next portless invocation (or `portless prune`) would clean it up.
- the typed-phrase confirmation is fine; we keep it.

New behavior:

1. After collecting listeners for `<port>`, partition owners into "claimed by a portless workload" (= the listener has a `WorkloadOwnsListener` edge from a `Workload { runtime: Portless }`) and "everyone else".
2. For portless-claimed listeners, plan **one** `Action { kind: ActionKind::PortlessStop, target: EntityRef::Process(<route.pid ProcessKey>) }` per workload (deduped: two listeners for the same hostname yield one action). The `ProcessKey` comes from `workload.source`, which Phase 2 populated from procfs. Implementation: SIGTERM the portless CLI pid via `nix::sys::signal::kill` with the same `ProcessKey` reuse-guard `execute_direct_action` already uses. Portless then `killTree`s the dev-server and removes the route.
3. For everyone else, fall back to today's `plan_direct_process`.
4. Mixed listeners on one port (procfs-only + portless) execute in two stages, both gated by the existing `ConfirmationPolicy::TypedPhrase { phrase: "free" }`: portless stop first, then re-snapshot after a 200 ms delay, then plan/execute direct SIGTERM only for remaining non-portless owners. Do not pre-plan direct actions for owners that the portless stop should already cover (avoids signaling the descendant twice).
5. Verification step (existing pattern from `run_free`): re-snapshot after action; if the listener is still present, emit `"portless stop did not free the port; the dev-server may have ignored SIGTERM. Run lazyadmin doctor for portless orphan/lock/proxy diagnostics or investigate manually."` Do not auto-escalate.
6. **Do not invoke `portless prune` from `free`.** `prune` is global ("kill every orphaned dev-server in routes.json") and racy (it picks up new orphans the moment any other portless CLI dies between scan and execute). It belongs as external doctor advice, not in a per-port stop path.

Dry-run output:

```
free port 3737: 1 listener, 1 owner action
  - stop portless app "zombie-test" (manager: portless)
    SIGTERM PID 2728641 (portless cli); portless will killTree the dev-server and remove the route
  - will not touch unrelated ports or use SIGKILL automatically
```

Confirmation policy: reuse `ConfirmationPolicy::TypedPhrase { phrase: "free" }`. Do not add a portless-specific phrase.

## Doctor checks

Add to `run_doctor` in `lazyadmin-cli`:

- `portless.state_dir` — does each resolved state dir (`$PORTLESS_STATE_DIR`, or otherwise `~/.portless` plus existing `/tmp/portless`) exist? `Info` if absent; `Ok` if present and `routes.json` is readable; `Warning` if present but `routes.json` is unreadable (EACCES) or unparseable.
- `portless.binary` — `which portless`. `Info` if missing; `Ok` if present (with version line in `summary`).
- `portless.orphan_routes` — count of `pid != 0` routes whose `/proc/<pid>` is dead. `Info` with hint `"run \`portless prune\` to clean up <N> orphaned route(s)"` when > 0.
- `portless.routes_lock` — directory `<state>/routes.lock` mtime age. portless's stale threshold is 10s; lazyadmin warns at 30s to leave headroom for the writer. `Warning` when held longer than 30s.
- `portless.proxy_daemon` — if `<state>/proxy.pid` exists, is the pid alive and listening on `<state>/proxy.port`? `Warning` if pidfile points at a dead pid (proxy probably crashed); `Ok` otherwise; check is skipped when no pidfile is present.

These checks are read-only and never touch the lock or routes file. This plan only adds `DoctorCheck` rows and hints. It does not add typed doctor remediations, `doctor --portless-prune-force`, or `ActionKind::PortlessPrune`.

## TUI surfacing

No new view. Existing rows gain a manager badge:

- `public` view and `ps` view, in human output only: append ` (portless: <hostname>.<tld>)` to listener rows whose `ListenerRow.manager_label = Some("portless")`.
- Inspector: a "Manager" field showing `portless · cli pid <P> · ~/.portless/routes.json:<hostname>` and, separately, `dev-server pid <Q>` for the actual listener owner.
- Doctor pane: any `portless.*` checks appear under a new `adapter:portless` group, beside the existing groups.

Render through projection helpers; no new layout code. The TUI consumes the same `manager_label` projection field that the human CLI does. `ps --json` and `public --json` continue to print filtered snapshots without `manager_label`.

## Skill update

`skills/lazyadmin-agent/SKILL.md` gets a short "Portless interop" section:

- For freeing a port, prefer `lazyadmin free <port>` over signaling raw pids — lazyadmin will pick the right manager dispatch and leave non-portless listeners alone.
- `portless prune` is an orphan reaper, **not** a per-route stop; never recommend it as a `free`-equivalent.
- When in doubt, `lazyadmin doctor` lists portless health (state dir, binary, orphans, lock age, proxy daemon) and may recommend running `portless prune` manually for orphans.

## Phases

### Phase 1 — adapter scaffold + read-only discovery

- [x] Create `crates/lazyadmin-adapter-portless` modeled on `lazyadmin-adapter-tracked` (no async runtime needed beyond `tokio::fs`).
- [x] Add the new crate to the workspace and add `lazyadmin-adapter-portless = { path = "../lazyadmin-adapter-portless" }` to `crates/lazyadmin-cli/Cargo.toml`.
- [x] Add `RuntimeKind::Portless` to `lazyadmin-core::model`.
- [x] Update every exhaustive `RuntimeKind` match, including TUI `runtime_label`, so the new variant is handled explicitly.
- [x] Wire the adapter into `build_snapshot_with_event_drops` in `lazyadmin-cli::main` next to procfs / systemd / container / tracked / project.
- [x] Unit tests over fixture `routes.json` (cases: empty, one live, one alias, one orphan, one with extra unknown field, one totally malformed, explicit `PORTLESS_STATE_DIR`, default `~/.portless`, legacy `/tmp/portless`). Assert per-case `workloads`, `managers`, and `warnings`.
- [x] Snapshot golden test: include one portless workload + one alias in a fixture under `testdata/snapshots/` and confirm `lazyadmin.snapshot.v1` round-trips.

Validation:

```bash
cargo metadata --format-version=1
cargo test -p lazyadmin-adapter-portless
cargo run -p lazyadmin-cli -- export --json | jq '[.workloads[] | select(.runtime == "portless")]'
cargo run -p lazyadmin-cli -- export --json | jq '[.warnings[] | select(.code | startswith("portless."))]'
```

### Phase 2 — correlation + manager-aware projection

- [x] In `lazyadmin-core::correlate`, build a parent→children index from `Graph.processes` (cheap O(n) pass over `Process.ppid`), reused across passes.
- [x] After the existing `classify_processes` pass and before/alongside conflict detection, add `correlate_portless`: for each portless workload, parse the route pid out of the workload's provenance, resolve it to a procfs `ProcessKey`, set `workload.source = Some(EntityRef::Process(cli_key))`, BFS descendants of that pid (depth cap 8) using the parent→children index, intersect with each listener's `owners`, and emit `Edge::WorkloadOwnsListener` (workload → listener) plus a `Provenance` line `"procfs-descendant-of portless cli pid <P>"`. Workloads gain the resolved descendant listener-owner `ProcessKey`s in `pids` at this stage; the CLI process key stays in `source`.
- [x] Add `lazyadmin-core::output::ListenerRow` (the module is currently a stub) with at least the existing row fields plus optional `manager_label: Option<String>` and `manager_detail: Option<String>`. Compute these from snapshot edges: a listener whose `WorkloadOwnsListener` workload has `runtime: Portless` gets `Some("portless")`, with route hostname and CLI pid detail where available. Human CLI `run_view` and TUI listener rendering consume this helper. Do not add `manager_label` to core `Listener` or snapshot JSON.
- [x] Tests: a procfs-fixture-plus-portless-fixture combined snapshot; assert the correlation resolves `workload.source` to the portless CLI `ProcessKey`, finds the descendant listener, emits the edge, back-fills descendant `pids`, and computes the row label.

Validation:

```bash
cargo test -p lazyadmin-core correlate
cargo run -p lazyadmin-cli -- export --json | jq '[.edges[] | select(.kind == "workload_owns_listener")]'
PORTLESS_STATE_DIR=/tmp/lazyadmin-portless-fixture cargo run -p lazyadmin-cli -- ps | rg 'portless:'
```

### Phase 3 — `free` dispatch (PortlessStop)

- [x] Add `ActionKind::PortlessStop` to `lazyadmin-core::actions`.
- [x] Refactor `run_free` into a small planner/executor split so unit tests can cover mixed portless/direct cases without shelling.
- [x] In `run_free`, walk listener edges and use the snapshot edges to discover whether the listener's workload (if any) has `runtime: Portless`. If yes, look up the portless workload's `source` `ProcessKey` and plan one `PortlessStop` per portless workload (deduped on workload id) with `target: EntityRef::Process(<route.pid ProcessKey>)`. Reuse the existing `Requirement::ProcessKeyMatch` and `Requirement::TypedPhrase { phrase: "free" }` so the existing confirmation flow is unchanged.
- [x] Implement `execute_portless_stop` that re-validates the `ProcessKey` against the live snapshot (refuse on PID-reuse mismatch), `nix::kill(SIGTERM)`s the route pid, then waits up to `action.timeout_ms` (default 5s) and re-snapshots. Return an `ActionResult` shaped exactly like `execute_direct_action` does.
- [x] Unit-test the planner over synthetic snapshots (portless-only, direct-only, mixed, two listeners same hostname, missing `workload.source`, alias workload).
- [x] Integration test gated behind `--features integration-portless`: spawn a synthetic `portless`-shaped subprocess (a small Rust binary that mimics portless's signal-handler-and-routes-file dance) bound to a free port in a temp `PORTLESS_STATE_DIR`, run `lazyadmin free <port>`, assert listener disappears and route is gone.
- [x] Document in CHANGELOG that `lazyadmin free <port>` will SIGTERM the portless CLI rather than the dev-server when a port is owned via portless.

Validation:

```bash
cargo test -p lazyadmin-cli run_free
cargo test -p lazyadmin-cli --features integration-portless free_portless_app
```

### Phase 4 — doctor + skill + docs

- [x] Implement the five doctor checks in `run_doctor`.
- [x] Keep orphan remediation as plain doctor hint text: `run portless prune to clean up <N> orphaned route(s)`. Do not add `ActionKind::PortlessPrune`, typed doctor remediation output, or a `doctor --portless-prune-force` flag in this plan.
- [x] `docs/portless-adapter.md` — public-facing description: state dir resolution, what we read, what we never write, fallback semantics, doctor surface, free dispatch behavior, alias handling.
- [x] `docs/spec.md` — add a one-line entry to the §10 (Discovery adapters) table.
- [x] `skills/lazyadmin-agent/SKILL.md` — portless interop note as outlined above.
- [x] CHANGELOG: `Adds read-only portless adapter, manager-aware free dispatch (SIGTERM portless CLI to free a portless-owned port), and portless health checks. New enum variants RuntimeKind::Portless and ActionKind::PortlessStop. Strict JSON-schema consumers should regenerate.`

### Phase 5 — release prep

- [x] Run the full validation block from `AGENTS.md`, including `cargo metadata --format-version=1`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, CLI JSON smokes, and one-filter-at-a-time TUI tests.
- [x] Manual dogfood with a real portless install:
  - `portless myapp -- node fake-dev-server.mjs` (start a route)
  - `lazyadmin ps` (expect `portless` label on `:<port>`)
  - `lazyadmin doctor` (expect `adapter:portless` group, all `Ok`)
  - `lazyadmin free <port>` (expect the route to disappear and the dev-server tree to die)
  - kill -9 the portless CLI to create an orphan; confirm `lazyadmin doctor` flags it and recommends `portless prune`.
- [x] Tag as a minor version bump (additive surface only).

Validation note: real-portless dogfood passed with `portless 0.11.1` using an isolated `PORTLESS_STATE_DIR` and `PORTLESS_PORT=18080`; synthetic `integration-portless` also exists. `lazyadmin ps` labeled the route, `lazyadmin doctor` reported `adapter:portless` checks, `lazyadmin free <port>` removed the route and listener, and a SIGKILL-created orphan produced the expected `portless prune` hint.

## Risks

- **Routes file schema drift.** portless's state is internal; the on-disk shape can change between 0.x releases. Mitigation: tolerant deserialization (`#[serde(default)]` on optional fields, drop on validation failure), explicit `Warning` rather than crash on parse failure, fixture-pinned tests so a schema change is caught locally.
- **Lock-free reads see partial writes.** portless writes via `fs.writeFileSync` (single `write(2)`) inside its `mkdir`-lock window. A torn read is rare but possible if the file grows past one page. Mitigation: parse failure → warning, return last-known nothing, retry on next snapshot. Acceptable because we never act on a single read; we always re-snapshot before mutating.
- **`route.pid` is the CLI, not the dev-server.** Already addressed by the procfs-descendant correlation. Risk: descendant walk is O(routes × procs), which is fine for typical dev workstations (tens of routes, hundreds of procs). Cap the BFS depth at 8 to prevent runaway costs in pathological process trees.
- **Alias routes (`pid == 0`).** They have no live owner. Documented above; surface as a workload but plan no action.
- **`portless prune` race.** `prune` is global. If the user runs it manually while `lazyadmin free <port>` is executing, the snapshot may transiently disagree with reality. Mitigation: `free` does not call prune; doctor only reports a hint.
- **PATH drift between snapshot and execute.** `PortlessStop` does not depend on the binary being on PATH (it signals a pid via `nix::kill`). Binary availability is doctor-only info in this plan.
- **Public JSON ambiguity.** Human/TUI rows get `manager_label`, but snapshot JSON does not. Mitigation: docs and validation explicitly assert scripts derive portless ownership from `edges` + `workloads`, and `ps --json` is not expected to expose `manager_label`.
- **Legacy state ambiguity.** `/tmp/portless` is read-only legacy state. Mitigation: only read it when no explicit `PORTLESS_STATE_DIR` is set and the directory already exists; tag all managers/workloads with state-dir provenance so users can tell which state file was used.
- **Cross-tool ownership ambiguity.** A user could `portless myapp -- lazyadmin run -- npm dev`. The npm grandchild has both a tracked-run record and a portless route. Resolution: portless wins for the listener-row `manager_label` because the descendant walk reaches it first; the tracked-run record is preserved as a secondary `provenance` line on the workload, not lost.
- **Proxy daemon vs. apps.** The portless proxy itself listens on `:443`/`:80`/`:1355`. `free 443` should *not* SIGTERM the daemon as a "portless app" — it has no route entry. The descendant-walk correlation only matches against routes, so the proxy listener naturally falls through to the existing direct-process path. Doctor surfaces the daemon separately.

## Deferred (not this plan)

- **PLAN-14 (proposed): portless route watch.** `notify`-based watcher on `~/.portless/routes.json` emitting `lazyadmin.discovery_event.v1` events on add/remove/changed. Snapshot polling stays authoritative.
- **PLAN-15 (proposed): `portless run` delegates to `lazyadmin run`.** Upstream-side change: `portless` shells out to `lazyadmin run --tag <appname> --detach -- <cmd>` so portless owns URL/proxy concerns and lazyadmin owns process supervision (`systemd-run --user --scope`). Requires a portless PR; pitch document lives in `wiki/portless-collab.md`, not in this repo.
- **PLAN-16 (proposed): `lazyadmin.discovery_event.v1` consumer SDK.** Public stream contract so portless and other tools can subscribe instead of polling.
- **Windows / WSL parity.** portless supports Windows; lazyadmin is Linux-first.

## Resolved decisions

- Use `RuntimeKind::Portless` (not `PortlessRoute`) to match the existing manager-family enum style (`Docker`, `Podman`, etc.).
- Add only `ActionKind::PortlessStop` in this plan. Defer `PortlessPrune` until lazyadmin has a typed doctor remediation/action surface.
- Keep CHANGELOG language as an additive feature note with explicit new enum variants for strict-schema consumers, no breaking-change banner.
- Keep `manager_label` out of `Snapshot.listeners` and `ps --json`; expose it through `lazyadmin-core::output::ListenerRow`, human CLI rendering, and TUI/headless TUI view models.
