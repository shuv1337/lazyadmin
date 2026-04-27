# JSON schema v1 fields and jq recipes

Common fields: `listeners[].port`, `bind_addr`, `protocol`, `exposure`, `owners[]`; `workloads[].id`, `runtime`, `pids[].pid`, `project.name`, `project.root`, `state`, `lazyadmin_run_id`, `restart_policy`, `actions[]`; `tracked_runs[].id`, `tag`, `cmd`, `cwd`, `started_at`, `state`; `warnings[]`.

v0.2 additive fields: `listeners[].dual_stack_state` may describe IPv6 wildcard confidence, and `metadata.events_dropped` may appear when event fan-in overflows. Discovery event streams use `lazyadmin.discovery_event.v1`; agents should treat them as refresh hints and continue using snapshots for authoritative state.

```bash
lazyadmin export --json | jq '.listeners[] | select(.port==3000)'
lazyadmin export --json | jq '.workloads[] | select(.runtime=="LazyadminTracked")'
lazyadmin export --json | jq '.listeners[] | select(.exposure=="lan_or_public" or .exposure=="public")'
lazyadmin diff /tmp/before.json - --json | jq '.removed.listeners[]?'
```
