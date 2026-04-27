# v0.2 acceptance validation

Environment: local Linux development checkout after PLAN-11 and PLAN-12. Container/systemd native event streams remain deferred per PLAN-11; TUI uses events as hints and snapshot polling as authoritative state.

| # | Criterion | Status | Command/test | Notes |
|---|---|---|---|---|
| 1 | sock_diag opt-in works | PARTIAL | `cargo test -p lazyadmin-adapter-procfs sockdiag` | Opt-in plumbing/fallback/provenance exists; native netlink enumeration deferred. |
| 2 | dual-stack proof present and honest | PARTIAL | `cargo test -p lazyadmin-adapter-procfs dualstack` | `confirmed_*` is only emitted when proven; current probes mostly remain `possible`. |
| 3 | `lazyadmin events --json` streams | PASS | `cargo run -p lazyadmin-cli -- events --once --json` | Procfs polling fan-in is available. |
| 4 | v0.1 view-models render | PASS | `cargo test -p lazyadmin-tui render_views` | TestBackend covers 120/90/70/50 widths. |
| 5 | Process Tree and Metrics views work | PARTIAL | `cargo test -p lazyadmin-tui process_tree metrics` | Snapshot-derived rendering, search filtering, PID-reuse-safe identity, and coarse snapshot-diff counters ship. Expand/collapse and richer inspector actions remain deferred. |
| 6 | keybinding overrides accepted and applied | PASS | `cargo test -p lazyadmin-core keybindings`; `cargo test -p lazyadmin-tui keybindings` | Config check exposes resolved keybindings; interactive dispatch uses the resolved map. |
| 7 | themes load and downgrade safely | PARTIAL | `cargo test -p lazyadmin-tui theme`; `lazyadmin config check --json` | Built-ins, explicit path loading, parse errors, color validation, and terminal fallback hints are covered. XDG theme-name lookup and missing-key inheritance are deferred. |
| 8 | copy-diagnostic works or falls back | PARTIAL | `cargo test -p lazyadmin-tui help_palette_open_copy` | Deterministic markdown file fallback helper exists; full interactive clipboard/open shell integration is deferred. |
| 9 | no JSON regressions | PASS | workspace tests + headless JSON | Changes are additive; snapshot schema remains v1. |

Release/tagging is still a user decision. Workspace version remains unchanged; this checkout should not be treated as v0.2 release-ready until the required validation matrix is green after review.
