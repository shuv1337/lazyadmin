# v0.1 acceptance validation

Environment: local Linux development checkout. Validation commands are best-effort where Docker/systemd/container fixtures are not guaranteed. `doctor --json` and snapshots should be captured during release QA.

| # | Criterion | Status | Command/test | Notes |
|---|---|---|---|---|
| 1 | `lazyadmin :3000` explains direct TCP listener with provenance | PARTIAL | manual local listener + `lazyadmin :3000` | Depends on live listener fixture. Core/CLI path exists. |
| 2 | `:5432` explains Compose Postgres restart policy | PARTIAL | Docker Compose fixture | Requires Docker/Compose environment. |
| 3 | systemd user and system services distinct | PARTIAL | `lazyadmin export --json` | Depends on systemd bus availability. |
| 4 | systemd socket activation without live PID | PARTIAL | systemd socket fixture | Manual release gate. |
| 5 | Public listeners easy to find | PASS | `lazyadmin public --json` | CLI/TUI view model exposes PUBLIC badge. |
| 6 | Compose service stopped from TUI with confirmation | PARTIAL | TUI action confirmation test | TUI routes through dispatcher; live Compose action requires environment. |
| 7 | Direct process SIGTERM and verify | PARTIAL | action safety tests | CLI executor path exists; live validation required. |
| 8 | Multiple owners never collapse fake owner | PASS | model/snapshot contract | Owners remain arrays/entity refs. |
| 9 | `free 5432` three owners atomically reports per-owner | PARTIAL | action safety tests | Requires multi-owner live fixture. |
|10| Permission denied visible | PASS | doctor/export contract | Warnings/doctor checks model permission gaps. |
|11| JSON export complete top-level sections | PASS | `cargo run -p lazyadmin-cli -- export --json` | Includes listeners/processes/workloads/managers/projects/tracked_runs/edges/provenance. |
|12| JSON diff meaningful | PASS | `cargo run -p lazyadmin-cli -- diff testdata/snapshots/empty.json testdata/snapshots/empty.json --json` | Diff command exists. |
|13| Doctor actionable structured severity | PASS | `lazyadmin doctor --json` | Structured report exists. |
|14| Secrets redacted by default | PASS | redaction tests | URL userinfo and secret-looking values covered by core redaction. |
|15| `lazyadmin run` wraps and stop descendants | PARTIAL | tracked-run integration | systemd-run support is environment dependent. |
|16| Verify after free reports auto-restart factually | PARTIAL | action report tests | Needs systemd/container restart fixture. |
|17| Two-tier filter and point-query bypass | PASS | TUI VM tests + CLI point queries | TUI hides system rows by default; point queries use CLI selector path. |
|18| Bracketed IPv6 selector parses | PASS | selector tests / `lazyadmin "tcp/[::1]:3000"` | CLI parser supports IPv6 brackets. |
|19| lazyadmin-agent skill ships and installs | PASS | `scripts/build-skill-tarball.sh` | Skill directory and install script present. |

Deferred/manual gaps are environmental, not intentional feature removals. CI keeps Linux integration behind `workflow_dispatch` because hosted runners may not provide clean user systemd/Docker/Podman coverage.
