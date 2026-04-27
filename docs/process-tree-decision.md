# Process Tree widget decision

Decision: render manually using Ratatui `Table` rows plus ASCII tree guides.

Rationale: v0.2 avoids an additional widget dependency while the process tree data model is still small and core-owned. Manual rendering is portable and testable with `TestBackend`. It currently renders snapshot-derived parent/child rows and preserves `ProcessKey` identity in the model; expand/collapse navigation and selected-process inspector details are deferred to v0.3.
