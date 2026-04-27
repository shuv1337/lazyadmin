# Process Tree widget decision

Decision: render manually using Ratatui `Table` rows plus ASCII tree guides.

Rationale: v0.2 avoids an additional widget dependency while the process tree data model is still small and core-owned. Manual rendering is portable, testable with `TestBackend`, and sufficient for stable selection by `ProcessKey`. A third-party tree widget can be revisited in v0.3 if richer keyboard navigation is needed.
