# Portless Adapter

`lazyadmin` reads portless route state so portless-owned dev servers can be displayed and freed through the right manager path.

## State Resolution

Resolution is read-only:

1. If `PORTLESS_STATE_DIR` is set, lazyadmin reads only that directory.
2. Otherwise lazyadmin reads `$HOME/.portless`.
3. If `/tmp/portless` already exists, lazyadmin also reads it as a legacy fallback.

Each readable state directory becomes one `Manager` with `kind: "portless"` and `scope: "user"`. Provenance records the state directory, binary path if `portless` is on `PATH`, and whether the directory is legacy.

## Route Semantics

Portless writes `routes.json` as entries shaped like:

```json
{ "hostname": "demo", "port": 3737, "pid": 2728641 }
```

`pid` is the portless CLI process, not the dev-server listener process. `pid: 0` is a static alias and has no live owner. lazyadmin represents aliases as portless workloads with no process action.

lazyadmin never writes `routes.json`, never takes the portless `routes.lock`, and never edits legacy state. Parse failures become `portless.routes_unparseable` warnings and are retried on the next snapshot.

## Correlation

The procfs adapter owns listener/process evidence. After adapter merge, core correlation resolves the portless CLI pid to a `ProcessKey`, walks its process descendants, and links descendant-owned listeners back to the portless workload with `workload_owns_listener` edges. Snapshot JSON does not add a listener `manager_label`; scripts should derive ownership from `edges` plus `workloads`.

Human `ps` / `public` output and the TUI use the projection helper to show rows such as `portless: demo cli pid 2728641`.

## Free Dispatch

For `lazyadmin free <port>`, portless-owned listeners are stopped by sending `SIGTERM` to the portless CLI process. Portless then runs its own cleanup path, kills the dev-server subtree, and removes the route. Direct-process listeners on the same port are handled separately after a re-snapshot.

`portless prune` is intentionally not used by `free`. It is a global orphan reaper, not a per-route stop command.

## Doctor Checks

`lazyadmin doctor` reports `adapter:portless` checks for:

- state directory and `routes.json` readability,
- `portless` binary availability and version when available,
- orphan route count with a manual `portless prune` hint,
- stale `routes.lock` directories,
- proxy daemon pidfile liveness and listener evidence when `proxy.pid` exists.
