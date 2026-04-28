# Process Tree widget decision

Decision: render manually using Ratatui `Table` rows plus ASCII tree guides.

Rationale: v0.2 avoids an additional widget dependency while the process tree data model is still small and core-owned. Manual rendering is portable and testable with `TestBackend`. It renders snapshot-derived parent/child rows, preserves `ProcessKey` identity in the model, supports expand/collapse from the `t` key path, and reuses the existing inspector for selected process details.
