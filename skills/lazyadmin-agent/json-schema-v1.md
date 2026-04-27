# JSON schema v1 fields and jq recipes

Common fields: `listeners[].port`, `bind_addr`, `protocol`, `exposure`, `owners[]`; `workloads[].id`, `runtime`, `pids[].pid`, `project.name`, `project.root`, `state`, `lazyadmin_run_id`, `restart_policy`, `actions[]`; `tracked_runs[].id`, `tag`, `cmd`, `cwd`, `started_at`, `state`; `warnings[]`.

```bash
lazyadmin export --json | jq '.listeners[] | select(.port==3000)'
lazyadmin export --json | jq '.workloads[] | select(.runtime=="LazyadminTracked")'
lazyadmin export --json | jq '.listeners[] | select(.exposure=="lan_or_public" or .exposure=="public")'
lazyadmin diff /tmp/before.json - --json | jq '.removed.listeners[]?'
```
