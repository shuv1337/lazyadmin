# PLAN-01 — Foundation, Core Model, CLI Skeleton, JSON Contract

Source: `lazyadmin-spec-v0_2.md` sections 7–9, 13, 17–18, 22–25, 28.  
Depends on: `PLAN-00-implementation-index-and-assumption-review.md`.  
Goal: create a compilable Rust workspace with the stable core model, config, redaction, selectors, snapshot/diff JSON, telemetry foundation, and non-mutating CLI shell.

## Implementation principles

- The core graph is the source of truth. CLI/TUI render projections only.
- JSON is a public API for agents and scripts; version it from the start.
- Every entity/action must carry provenance, confidence, and stable IDs where possible.
- Redaction and telemetry are not optional polish; they are foundational.

## Target repo layout

```text
Cargo.toml
crates/
  lazyadmin-core/
    Cargo.toml
    src/
      lib.rs
      model/
      graph/
      snapshot/
      diff/
      correlate/
      actions/
      config/
      redact/
      selector/
      telemetry/
      output/
  lazyadmin-cli/
    Cargo.toml
    src/main.rs
  lazyadmin-tui/                 # stub only in this plan
    Cargo.toml
    src/lib.rs
  lazyadmin-adapter-procfs/      # stub only in this plan
  lazyadmin-adapter-systemd/     # stub only in this plan
  lazyadmin-adapter-container/   # stub only in this plan
  lazyadmin-adapter-project/     # stub only in this plan
  lazyadmin-adapter-tracked/     # stub only in this plan
docs/
  schema/
  spec.md                        # copy or symlink from lazyadmin-spec-v0_2.md after decision
testdata/
  snapshots/
```

## Dependencies to add initially

Root/workspace:

```toml
[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde", "clock"] }
clap = { version = "4", features = ["derive", "env"] }
color-eyre = "0.6"
directories = "6"
futures = "0.3"
indexmap = { version = "2", features = ["serde"] }
insta = { version = "1", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
strum = { version = "0.27", features = ["derive"] }
thiserror = "2"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "signal", "process", "fs"] }
toml = "0.9"
tracing = "0.1"
tracing-error = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
uuid = { version = "1", features = ["serde", "v7"] }
```

Add low-level adapter dependencies in later plans only when used.

## Phase 1 — Workspace and crate skeleton

- [x] Create the Cargo workspace and initial crates listed above.
- [x] Configure common lint posture:
  - [x] `#![forbid(unsafe_code)]` in crates except low-level modules that later justify scoped `unsafe`.
  - [x] `#![deny(missing_debug_implementations)]` in core once models settle.
  - [x] Clippy CI with `-D warnings` after initial churn.
- [x] Add `.gitignore` for Rust build artifacts and local runtime outputs.
- [x] Add `README.md` with current development status and link to plan docs.
- [x] Copy the product spec into `docs/spec.md` or leave `lazyadmin-spec-v0_2.md` as source and link it from docs; do not fork divergent specs silently.

Validation:

```bash
cargo metadata --format-version=1
cargo check --workspace
```

## Phase 2 — Core model types

Implement in `crates/lazyadmin-core/src/model/`.

- [x] Define IDs as newtypes, not raw strings:
  - [x] `ListenerId`
  - [x] `ProcessId` / `ProcessRef`
  - [x] `WorkloadId` / `WorkloadRef`
  - [x] `ManagerId` / `ManagerRef`
  - [x] `ProjectId` / `ProjectRef`
  - [x] `RunId`
  - [x] `ActionId`
  - [x] `EntityRef`
- [x] Define enums from the spec:
  - [x] `RuntimeKind`
  - [x] `Confidence`
  - [x] `Exposure`
  - [x] `Protocol`
  - [x] `AddressFamily`
  - [x] `ListenerState`
  - [x] `WorkloadState`
  - [x] `ManagerScope`
  - [x] `PermissionState`
  - [x] `DangerLevel`
  - [x] `RestartPolicySource`
  - [x] `WarningSeverity`
- [x] Define primary structs:
  - [x] `Listener`
  - [x] `Process`
  - [x] `ProcessKey { pid, boot_id, start_time_ticks }`
  - [x] `Workload`
  - [x] `RestartPolicy`
  - [x] `Manager`
  - [x] `Project`
  - [x] `TrackedRun`
  - [x] `Edge`
  - [x] `Provenance`
  - [x] `Warning`
  - [x] `Snapshot`
- [x] Use `serde(rename_all = "snake_case")` or an explicitly documented casing convention consistently.
- [x] Add `schema_version` constants:
  - [x] `lazyadmin.snapshot.v1`
  - [x] `lazyadmin.diff.v1`
  - [x] `lazyadmin.doctor.v1`

Acceptance:

- [x] A fixture snapshot can serialize/deserialize round-trip without loss.
- [x] IDs cannot be accidentally mixed across entity kinds at compile time.
- [x] `ProcessKey` is used anywhere process identity is persisted beyond a single scan.

## Phase 3 — Graph and discovery adapter contracts

Implement in `crates/lazyadmin-core/src/graph/` and `snapshot/`.

- [x] Add `Graph` container indexes:
  - [x] entity by ID,
  - [x] listeners by `(netns, protocol, family, bind_addr, port, socket_inode)`,
  - [x] processes by `ProcessKey`,
  - [x] workloads by manager/runtime/source IDs,
  - [x] edges by source/target.
- [x] Add `DiscoveryAdapter` trait with `watch()` returning `Option<BoxStream<'static, DiscoveryEvent>>` as in the spec.
- [x] Add `DiscoveryOutput` with managers/processes/listeners/workloads/projects/tracked_runs/edges/warnings.
- [x] Add merge/build APIs:
  - [x] `SnapshotBuilder::from_adapter_outputs(Vec<DiscoveryOutput>)`.
  - [x] Preserve all provenance; do not collapse evidence during merge.
  - [x] Confidence aggregation: listener confidence is max confidence across provenance unless conflict rules produce explicit warnings.
- [x] Add placeholder orchestrator that can run no adapters and return an empty snapshot.

Telemetry:

- [x] Add spans for `snapshot.build`, `adapter.discover`, `graph.merge`, and `graph.correlate` even before adapters exist.
- [x] Include fields: `adapter`, `entity_counts`, `duration_ms`, `result`, `error.class` when available.

Validation:

```bash
cargo test -p lazyadmin-core graph snapshot
```

## Phase 4 — Config loading

Implement in `crates/lazyadmin-core/src/config/`.

- [x] Define `Config` matching spec section 17:
  - [x] `ui`
  - [x] `ports`
  - [x] `actions`
  - [x] `redaction`
  - [x] `adapters.sockets`
  - [x] `adapters.systemd`
  - [x] `adapters.container`
  - [x] `adapters.tracked`
  - [x] `projects`
  - [x] `visibility.system_service_denylist`
- [x] Implement default config values from the spec.
- [x] Load from:
  - [x] `$XDG_CONFIG_HOME/lazyadmin/config.toml`
  - [x] `~/.config/lazyadmin/config.toml`
- [x] Expand `~`, `$XDG_STATE_HOME`, `$XDG_RUNTIME_DIR`, and explicit env vars only in documented path fields.
- [x] Validate config:
  - [x] refresh interval bounds,
  - [x] common port ranges,
  - [x] known enum strings,
  - [x] no duplicate project roots after normalization.
- [x] Add `lazyadmin config check --json` CLI route.

Validation:

```bash
cargo test -p lazyadmin-core config
cargo run -p lazyadmin-cli -- config check --json
```

## Phase 5 — Redaction primitives

Implement in `crates/lazyadmin-core/src/redact/`.

- [x] Redact key/value pairs by case-insensitive key patterns:
  - `token`, `secret`, `password`, `passwd`, `pwd`, `apikey`, `api_key`, `authorization`, `credential`, `session`, `cookie`, `private_key`.
- [x] Redact CLI args shaped like:
  - [x] `--token value`
  - [x] `--token=value`
  - [x] `TOKEN=value`
- [x] Redact URL userinfo: `scheme://user:pass@host` -> `scheme://user:<redacted>@host`.
- [x] Define `Redacted<T>` or equivalent wrapper for sensitive display values.
- [x] Ensure diagnostic copy and JSON intended for sharing use redacted values by default.
- [x] Add explicit reveal type that requires confirmation in action/UI layers later.

Tests:

- [x] env var redaction,
- [x] cmdline redaction,
- [x] URL userinfo,
- [x] mixed-case keys,
- [x] false-positive minimization for harmless words.

Validation:

```bash
cargo test -p lazyadmin-core redact
```

## Phase 6 — Selector parser

Implement in `crates/lazyadmin-core/src/selector/`.

Supported selectors:

- [x] `:3000`
- [x] `127.0.0.1:3000`
- [x] `[::1]:3000`
- [x] `[::]:3000`
- [x] `tcp/:3000`
- [x] `tcp/127.0.0.1:3000`
- [x] `tcp/[::1]:3000`
- [x] `udp/[::]:5353`
- [x] `unix:///tmp/app.sock`
- [x] `pid:42420`
- [x] `unit:dev-api.service`
- [x] `unit:dev-api.socket`
- [x] `container:localdb-postgres-1`
- [x] `compose:localdb/postgres`
- [x] `project:acme/web`
- [x] `project:~/src/acme/web`
- [x] `run:r-7f9a`
- [x] `tag:acme-web`

Parser requirements:

- [x] Reject unbracketed IPv6 host/port selectors.
- [x] Reject bracketed IPv6 without a port.
- [x] Preserve protocol when specified; default to `Any` protocol for bare port until query resolution.
- [x] Produce structured parse errors with correction hints.
- [x] Unit selectors should preserve suffix so later resolution can route `.socket` vs `.service`.

Validation:

```bash
cargo test -p lazyadmin-core selector
```

## Phase 7 — JSON snapshot and diff contract

Implement in `snapshot/`, `diff/`, and `docs/schema/`.

- [x] Define `Snapshot` JSON shape from spec section 18.1.
- [x] Define `Diff` JSON shape with:
  - [x] added/removed/changed listeners,
  - [x] added/removed/changed workloads,
  - [x] owner changes,
  - [x] warnings changes,
  - [x] action verification-friendly summaries.
- [x] Add JSON schema documentation:
  - [x] `docs/schema/snapshot-v1.md`
  - [x] `docs/schema/diff-v1.md`
  - [x] optional machine-readable `*.schema.json` if team chooses.
- [x] Add golden tests using `insta` for representative snapshots/diffs.
- [x] Add semantic compatibility tests that fail on accidental field removal/rename.
- [x] Implement CLI commands:
  - [x] `lazyadmin export --json` returns empty-but-valid snapshot until adapters land.
  - [x] `lazyadmin diff <before> <after>` compares two snapshot files.
  - [x] `lazyadmin diff <before> -` compares `<before>` to current snapshot.
  - [x] `lazyadmin diff --json` emits `lazyadmin.diff.v1`.

Validation:

```bash
cargo test -p lazyadmin-core snapshot diff
cargo run -p lazyadmin-cli -- export --json | jq .schema_version
cargo run -p lazyadmin-cli -- diff testdata/snapshots/empty.json testdata/snapshots/empty.json --json
```

## Phase 8 — CLI skeleton and output conventions

Implement in `crates/lazyadmin-cli`.

- [x] Add Clap command tree from spec section 13:
  - [x] default TUI route placeholder,
  - [x] point query route placeholder,
  - [x] `port`, `free`, `ps`, `public`, `conflicts`, `projects`, `logs`, `doctor`, `export`, `diff`, `run`, `runs`, `pause-restart`, `resume-restart`, `config check`.
- [x] Commands that are not implemented yet should return `EX_UNAVAILABLE`-style error with clear message, not panic.
- [x] Add global flags:
  - [x] `--json`, where appropriate,
  - [x] `--brief`, for point queries,
  - [x] `--config PATH`,
  - [x] `--log-format text|json`,
  - [x] `-v/--verbose` repeated.
- [x] Initialize `color-eyre` and `tracing_subscriber` once in `main`.
- [x] Human output must be generated from typed output structs, not hand-built throughout command handlers.

Validation:

```bash
cargo run -p lazyadmin-cli -- --help
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- diff --help
```

## Phase 9 — Foundation CI

- [x] Add GitHub Actions Linux workflow:
  - [x] `cargo fmt --all -- --check`
  - [x] `cargo clippy --workspace --all-targets -- -D warnings`
  - [x] `cargo test --workspace`
  - [x] `cargo doc --workspace --no-deps`
- [x] Add optional nightly or scheduled job for integration tests later.
- [x] Cache Cargo registry/build artifacts.
- [x] Upload test snapshots on failure if practical.

## Done criteria

- [x] `cargo check --workspace` succeeds.
- [x] Core models serialize/deserialize stable JSON.
- [x] Selector parser covers all grammar examples and rejection cases.
- [x] Config defaults match the spec and can be overridden by TOML.
- [x] Redaction tests pass for env/cmdline/URL cases.
- [x] `lazyadmin export --json` and `lazyadmin diff --json` exist and use schema versions.
- [x] Telemetry initialization and core spans exist.
- [x] CI runs format, clippy, tests, and docs on Linux.

## Handoff notes for next plan

`PLAN-02` can start once the graph, adapter traits, snapshot builder, config, selector parser, and CLI skeleton compile. Discovery adapters should return `DiscoveryOutput` only; do not let them construct final user-facing rows directly.
