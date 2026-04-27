# lazyadmin.diff.v1

Public JSON produced by `lazyadmin diff --json`.

Required top-level fields: `schema_version`, `generated_at`, `listeners`, `workloads`, `owner_changes`, `warning_changes`, `summaries`.

`listeners`, `workloads`, and `warning_changes` each contain `added`, `removed`, and `changed` arrays.
