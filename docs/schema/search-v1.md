# `lazyadmin.search.v1`

Global search is a read-only projection built by `lazyadmin_runtime::view_model::search`.
The same JSON shape is returned by `lazyadmin search <query> --json`, Web
`GET /api/search?q=<query>`, and headless TUI `view_model.search`.

Top-level fields:

- `schema_version`: always `lazyadmin.search.v1`.
- `query`: original `raw`, trimmed `normalized`, and internally tagged `kind`.
- `listeners`, `processes`, `workloads`, `projects`, `managers`, `rail_views`: result groups.
- `strategy_hint`: UI hint such as `text query`, `port :5432`, `port :54 (prefix)`, or `pid 12345`.
- `fell_back_to_prefix`: true only when a port query found no exact listener and used port-prefix matching.
- `elapsed_ms`: matcher runtime in milliseconds.

Each group has `total`, `returned`, `truncated`, and `hits`. `total` is matches
before limit, `returned` is serialized hit count, and `truncated` means
`returned < total`.

## Query Kinds

```json
{ "type": "empty" }
{ "type": "text", "text": "hermes" }
{ "type": "port", "port": 5432 }
{ "type": "pid", "pid": 99999 }
```

Pure digits that fit `u16` are port queries. Pure digits that do not fit `u16`
but fit positive `i32` are PID queries. Empty strings are `empty`; everything
else is `text`.

## Hit Fields

Listener hit:

- `id`, `port`, `bind`, `protocol`, `exposure`
- `owner_label`, `workload_labels`, `project_label`
- `score`, `matched_indices`, `is_system`

Process hit:

- `key`, `pid`, `user`, `exe_or_argv0`, `cmdline_compact`, `cwd`
- `score`, `matched_indices`, `is_system`

Workload hit:

- `id`, `display_name`, `runtime`, `project_label`, `manager_label`
- `listener_count`, `pid_count`, `score`, `matched_indices`

Project hit:

- `id`, `name`, `root`, `package_manager`, `git_remote`
- `score`, `matched_indices`

Manager hit:

- `id`, `name`, `kind`, `scope`, `available`
- `score`, `matched_indices`

Rail view hit:

- `id`, `label`, `score`, `matched_indices`

`matched_indices` is a list of matched character indices from the fuzzy matcher,
not coalesced ranges.

## Examples

The non-empty examples below are compact fragments. Omitted result groups use the
same group envelope (`total`, `returned`, `truncated`, `hits`) shown in the empty
query example.

Empty query:

```json
{
  "schema_version": "lazyadmin.search.v1",
  "query": { "raw": "", "normalized": "", "kind": { "type": "empty" } },
  "listeners": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "processes": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "workloads": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "projects": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "managers": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "rail_views": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "strategy_hint": "",
  "fell_back_to_prefix": false,
  "elapsed_ms": 0
}
```

Text query:

```json
{
  "schema_version": "lazyadmin.search.v1",
  "query": { "raw": "hermes", "normalized": "hermes", "kind": { "type": "text", "text": "hermes" } },
  "listeners": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "processes": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "workloads": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "projects": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "managers": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "rail_views": { "total": 0, "returned": 0, "truncated": false, "hits": [] },
  "strategy_hint": "text query",
  "fell_back_to_prefix": false,
  "elapsed_ms": 0
}
```

Port exact query:

```json
{
  "query": { "raw": "5432", "normalized": "5432", "kind": { "type": "port", "port": 5432 } },
  "listeners": {
    "total": 1,
    "returned": 1,
    "truncated": false,
    "hits": [{
      "id": "tcp:127.0.0.1:5432:123",
      "port": 5432,
      "bind": "127.0.0.1:5432",
      "protocol": "tcp",
      "exposure": "loopback",
      "owner_label": "postgres",
      "workload_labels": [],
      "project_label": null,
      "score": 10000,
      "matched_indices": [0, 1, 2, 3],
      "is_system": false
    }]
  },
  "strategy_hint": "port :5432",
  "fell_back_to_prefix": false
}
```

Port prefix query:

```json
{
  "query": { "raw": "54", "normalized": "54", "kind": { "type": "port", "port": 54 } },
  "listeners": { "total": 2, "returned": 2, "truncated": false, "hits": [] },
  "strategy_hint": "port :54 (prefix)",
  "fell_back_to_prefix": true
}
```

PID query:

```json
{
  "query": { "raw": "99999", "normalized": "99999", "kind": { "type": "pid", "pid": 99999 } },
  "processes": { "total": 1, "returned": 1, "truncated": false, "hits": [] },
  "strategy_hint": "pid 99999",
  "fell_back_to_prefix": false
}
```
