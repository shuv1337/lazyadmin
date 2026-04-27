# lazyadmin Project Notes

## Current state

This repository now contains the PLAN-01 Rust workspace foundation for `lazyadmin`:

- `Cargo.toml` — workspace root (`resolver = "2"`) with shared dependencies.
- `crates/lazyadmin-core` — core models, graph/discovery contracts, config loader, redaction, selectors, snapshot/diff JSON contract, and telemetry primitives.
- `crates/lazyadmin-cli` — Clap command skeleton; `export`, `diff`, and `config check` are available while other commands return an EX_UNAVAILABLE-style error.
- `crates/lazyadmin-tui` remains a stub; procfs, systemd, tracked, container, and project adapters now provide read-only discovery foundations.
- `lazyadmin-spec-v0_2.md` — source specification for the Linux-first Rust + Ratatui local runtime control plane.
- `docs/spec.md` — symlink to the source spec; do not fork divergent specs silently.
- `docs/schema/` and `testdata/snapshots/` — initial public JSON contract docs and fixtures.
- `PLAN-*.md` — implementation-ready planning documents derived from the spec and assumption review.

## Intended architecture

`lazyadmin` is planned as a Linux-first Rust workspace with:

- `crates/lazyadmin-core` for normalized graph models, correlation, actions, config, redaction, JSON schemas, diffs, and telemetry primitives.
- `crates/lazyadmin-cli` for CLI parsing and human/JSON output, including `export`, `diff`, `config check`, tracked-run commands, and read-only `ps`/`public`/`conflicts`/`projects` views.
- `crates/lazyadmin-tui` for the Ratatui interface.
- Adapter crates for procfs/sockets, systemd, containers, projects, and tracked runs.
- `skills/lazyadmin-agent` for AI-agent integration guidance.

## Important validated assumptions and caveats

- The spec's `systemd-run --user --scope` path needs an implementation spike before relying on detached-run and journal-log behavior. The plan keeps scopes as the v0.1 default candidate, but requires proving or downgrading claims.
- Bollard is confirmed as a Docker Engine API Rust client, but the plan does not assume automatic Podman socket discovery without a spike. Probe Docker and Podman sockets explicitly.
- `GetUnitByPIDFD`, `GetUnitByPID`, `GetUnitByControlGroup`, `StopUnit`, `RestartUnit`, `KillUnit`, `StartTransientUnit`, and unit-listing methods exist in systemd's D-Bus Manager interface, but runtime availability varies by systemd version.
- Exact IPv6 dual-stack (`IPV6_V6ONLY`) detection is not proven from `/proc/net` alone. v0.1 should label it best-effort unless the adapter can prove the per-socket option.
- `pause-restart` systemd semantics are unresolved. Do not hard-code masking as the only solution until the spike in `PLAN-04-actions-logs-safety-doctor.md` is complete.

## Development standards

- Telemetry is day-zero: every adapter scan, correlation pass, action plan/execution, CLI command, TUI refresh, and log stream should emit structured `tracing` spans/events with stable IDs where possible.
- Prefer safe, reversible actions. All mutating runtime actions need dry-run output, explicit confirmation policy, structured result reporting, and post-action verification.
- JSON output is a public contract for scripts and agents. Add schema/golden tests before changing it.
- Keep human output and TUI view models as projections of the core graph; do not duplicate ownership/correlation logic in UI crates.

## Validation commands

Run these before handing off foundation changes:

```bash
cargo metadata --format-version=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p lazyadmin-cli -- --help
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- ps --json
cargo run -p lazyadmin-cli -- public --json
cargo run -p lazyadmin-cli -- conflicts --json
cargo run -p lazyadmin-cli -- projects --json
cargo run -p lazyadmin-cli -- diff testdata/snapshots/empty.json testdata/snapshots/empty.json --json
cargo run -p lazyadmin-cli -- config check --json
```

Later plans may add Linux integration tests such as:

```bash
cargo test --workspace --features integration-linux -- --ignored
```

Update this file when the workspace layout, commands, or operational caveats change.
