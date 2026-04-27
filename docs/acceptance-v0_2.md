# v0.2 acceptance validation

Environment: local Linux development checkout after PLAN-11 and PLAN-12. Container/systemd native event streams remain deferred per PLAN-11; TUI uses events as hints and snapshot polling as authoritative state.

| # | Criterion | Status | Command/test | Notes |
|---|---|---|---|---|
| 1 | sock_diag opt-in works | PARTIAL | `cargo test -p lazyadmin-adapter-procfs sockdiag` | Opt-in plumbing/fallback/provenance exists; native netlink enumeration deferred. |
| 2 | dual-stack proof present and honest | PARTIAL | `cargo test -p lazyadmin-adapter-procfs dualstack` | `confirmed_*` is only emitted when proven; current probes mostly remain `possible`. |
| 3 | `lazyadmin events --json` streams | PASS | `cargo run -p lazyadmin-cli -- events --once --json` | Procfs polling fan-in is available. |
| 4 | v0.1 view-models render | PASS | `cargo test -p lazyadmin-tui render_views` | TestBackend covers 120/90/70/50 widths. |
| 5 | Process Tree and Metrics views work | PASS | `cargo test -p lazyadmin-tui process_tree metrics` | Core snapshot-derived models and rendering ship. |
| 6 | keybinding overrides accepted | PASS | `cargo test -p lazyadmin-core keybindings` | Config check exposes resolved keybindings. |
| 7 | themes load and downgrade safely | PASS | `cargo test -p lazyadmin-tui theme` | Built-ins and color validation covered. |
| 8 | copy-diagnostic works or falls back | PASS | `cargo test -p lazyadmin-tui help_palette_open_copy` | Clipboard fallback writes markdown under state dir. |
| 9 | no JSON regressions | PASS | workspace tests + headless JSON | Changes are additive; snapshot schema remains v1. |

Release/tagging is still a user decision. Workspace version remains unchanged unless release readiness is explicitly approved after final review.
