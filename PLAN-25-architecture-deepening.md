# PLAN-25 - Architecture Deepening

## Status

Draft plan created from the architecture review on 2026-05-25.

Reality update on 2026-06-23: the architecture-deepening work has largely
landed. The GitHub issue breakdown at the bottom of this plan has no remaining
open issues in the live repository, and the current checkout includes the
runtime relation lens, shared listener table projection, live snapshot feed,
TUI interaction test seam, reusable free-port planner, architecture docs, and
agent guidance updates.

Follow-up work completed during this reality update:

- [x] Added live-feed tests for event-hint refresh and dropped-event propagation.
- [x] Moved confirmation handling, search-mode transitions, and listener sort
  selection preservation into `crates/lazyadmin-tui/src/interaction.rs` instead
  of leaving the interaction module as a thin wrapper.
- [x] Updated `AGENTS.md` to include the shipped
  `/api/views/listeners` Web API route.
- [x] Ran `cargo fmt --all`, `cargo test -p lazyadmin-runtime`,
  `cargo test -p lazyadmin-tui`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo run -p lazyadmin-cli -- web --port 0 --no-open` under a timeout,
  and `cargo test -p lazyadmin-cli --features integration-portless free_portless_app`.

Validation completed in this pass:

- [x] Full workspace test validation from Milestone 6.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] Browser smoke for `lazyadmin web --port 0 --no-open`.
- [x] Integration-portless free-port test.

Implementation status by milestone:

- Milestone 0: Partially complete. Focused validation has been rerun, but the
  working tree is not clean because this plan, `implementation-notes.html`,
  and `goals/` are untracked.
- Milestone 1: Complete. `SnapshotRelations` exists, is tested, and is used by
  digest, search, inspector, and listener-table projections.
- Milestone 2: Complete for the primary listener table path. Runtime owns
  listener rows/filter/sort, TUI consumes the runtime projection, and Web uses
  `/api/views/listeners`. Some legacy summary-view helper code remains in the
  TUI for non-primary projections.
- Milestone 3: Complete. TUI and Web consume `spawn_live_snapshot_feed`, events
  remain hints, polling remains authoritative, and drop counts are now tested.
- Milestone 4: Complete for the planned first reducer. Search, confirmation,
  and sort-preserve-selection behavior are handled through the interaction
  module and covered by direct interaction tests. Larger render/state file
  splits remain intentionally deferred.
- Milestone 5: Complete for pure action planning. Core owns free-port planning,
  dry-run text, and inspector preview actions; CLI still owns host mutation,
  confirmation, process-key validation, rescans, and post-action verification.
- Milestone 6: Complete for PLAN-25 scope. Architecture docs and AGENTS
  guidance now match the shipped modules and routes, workspace tests pass,
  clippy passes, Web smoke reaches the loopback listening state, and the
  integration-portless free-port test passes.

This plan covers five deepening opportunities:

1. Shared listener table projection.
2. Snapshot relation lens.
3. Single live snapshot feed.
4. TUI interaction core.
5. Safe action planning module.

The intent is to improve locality and leverage without changing public snapshot,
doctor, search, or diff JSON contracts unless a later issue explicitly opts into
a versioned contract change.

## Context

lazyadmin is a Linux-first local runtime control plane with three main user
surfaces:

- CLI commands in `crates/lazyadmin-cli/src/main.rs`.
- Ratatui interface in `crates/lazyadmin-tui/src/lib.rs`.
- Read-only Web UI in `crates/lazyadmin-web/src/lib.rs` and
  `crates/lazyadmin-web/static/app.js`.

The project has already moved several shared projections into
`crates/lazyadmin-runtime/src/view_model/`:

- `digest.rs`
- `doctor_groups.rs`
- `header_pip.rs`
- `inspector.rs`
- `search.rs`

That direction is correct, but several deep behaviours still leak into caller
modules. This plan continues the same architecture: core owns normalized
snapshot contracts, runtime owns shared projection and live-observation
behaviour, and UI crates render view models.

## Non-goals

- Do not change `lazyadmin.snapshot.v1`, `lazyadmin.diff.v1`,
  `lazyadmin.search.v1`, or `lazyadmin.discovery_event.v1` shapes as part of
  this plan.
- Do not add Web UI runtime mutation routes.
- Do not introduce a Web bundler or frontend framework.
- Do not split files mechanically without moving behaviour behind a deeper
  interface.
- Do not replace the existing adapter model.

## Related Issues And Plans

- GitHub issue #13: UX overhaul tracking.
- GitHub issue #23: Inspector ActionPreview dry-run output.
- GitHub issue #24: Reconcile global search and per-view filters.
- GitHub issue #26: PLAN-24 global fuzzy search closeout.
- `PLAN-15-ux-overhaul.md`: runtime projection foundation and UX overhaul.
- `PLAN-16-listener-sort-hardening.md`: listener sort parity caveat and
  explicit note that a later runtime-view-model plan can consolidate sorting.
- `PLAN-24-global-search.md`: shared runtime search contract.
- `docs/architecture.md`: core-first architecture and projection model.
- `docs/discovery-events-decision.md`: events are hints, snapshots remain
  authoritative, and long-lived consumers should use one drop counter source.
- `docs/action-safety.md`: current conservative action safety model.

## Architectural Language

This plan uses the codebase-architecture vocabulary:

- A **Module** is anything with an interface and an implementation.
- An **Interface** is everything callers must know to use the module correctly.
- A **Seam** is where an interface lives.
- An **Adapter** is a concrete thing satisfying an interface at a seam.
- **Depth** means a small interface hides a meaningful amount of behaviour.
- **Leverage** is what callers get from that depth.
- **Locality** is what maintainers get from that depth.

The deletion test for every new module is: if the module were deleted, would
complexity reappear across TUI, Web, CLI, and tests? If yes, the module is
earning its keep. If no, the module is shallow and should not be introduced.

## Milestone 0 - Baseline And Guardrails

Goal: capture the current state and prevent accidental public-contract churn.

Tasks:

- [ ] Confirm the working tree is clean before starting:

  ```bash
  git status --short
  ```

- [ ] Run baseline focused checks:

  ```bash
  cargo test -p lazyadmin-runtime
  cargo test -p lazyadmin-web
  cargo test -p lazyadmin-tui render_views
  cargo test -p lazyadmin-cli --test cli_smoke
  ```

- [ ] Capture a before/after JSON smoke set for public contracts:

  ```bash
  cargo run -p lazyadmin-cli -- export --json
  cargo run -p lazyadmin-cli -- doctor --json
  cargo run -p lazyadmin-cli -- overview --json
  cargo run -p lazyadmin-cli -- search hermes --json
  cargo run -p lazyadmin-cli -- tui --headless --json
  ```

- [ ] Keep all refactors behaviour-preserving until a specific slice's
  acceptance criteria says otherwise.
- [ ] When changing a shared projection, add tests in
  `crates/lazyadmin-runtime/src/view_model/` first.

Validation:

- [ ] Baseline commands are green before behaviour moves.
- [ ] Any failing baseline is documented before implementation starts.

## Milestone 1 - Snapshot Relation Lens

Opportunity: snapshot relation knowledge is repeated in digest, search,
inspector, TUI rows, and core output helpers.

Relevant files:

- `crates/lazyadmin-runtime/src/view_model/digest.rs`
- `crates/lazyadmin-runtime/src/view_model/search.rs`
- `crates/lazyadmin-runtime/src/view_model/inspector.rs`
- `crates/lazyadmin-tui/src/lib.rs`
- `crates/lazyadmin-core/src/output/mod.rs`

Problem:

View modules repeatedly look up owners, projects, managers, related listeners,
system ownership, and listener bind labels by manually walking `Snapshot`.
Each caller must know details of the graph. That makes each caller's interface
nearly as complex as the implementation it uses.

Solution:

Add a runtime-only relation lens module, tentatively
`crates/lazyadmin-runtime/src/view_model/relations.rs`, that builds indexed
snapshot facts for view projections.

Candidate interface:

```rust
pub struct SnapshotRelations<'a> { /* internal indexes */ }

impl<'a> SnapshotRelations<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Self;
    pub fn listener_bind(&self, listener: &Listener) -> String;
    pub fn listener_owner_label(&self, listener: &Listener) -> String;
    pub fn listener_owner_pid(&self, listener: &Listener) -> Option<i32>;
    pub fn listener_project_label(&self, listener: &Listener) -> Option<String>;
    pub fn listener_manager_label(&self, listener: &Listener) -> Option<String>;
    pub fn listener_is_system(&self, listener: &Listener) -> bool;
    pub fn listener_related_by_pid(&self, listener: &Listener) -> Vec<RelatedListener>;
}
```

The exact interface should be trimmed during implementation. Do not expose
helpers just because an internal implementation already has them.

Tasks:

- [ ] Create `relations.rs` under `crates/lazyadmin-runtime/src/view_model/`.
- [ ] Add relation tests over `Snapshot::empty()` and
  `testdata/snapshots/busy.json`.
- [ ] Move only duplicated relation facts first:
  - [ ] Listener bind formatting.
  - [ ] Owner label and owner PID lookup.
  - [ ] Project label lookup.
  - [ ] Manager label lookup.
  - [ ] System listener detection.
- [ ] Update `digest.rs` to consume `SnapshotRelations`.
- [ ] Update `search.rs` to consume `SnapshotRelations`.
- [ ] Update `inspector.rs` to consume `SnapshotRelations`.
- [ ] Leave `crates/lazyadmin-core/src/output/mod.rs` unchanged unless a
  specific core-only projection still needs it.

Validation:

- [ ] `cargo test -p lazyadmin-runtime`
- [ ] `cargo test -p lazyadmin-cli --test cli_smoke`
- [ ] `cargo run -p lazyadmin-cli -- overview --json`
- [ ] `cargo run -p lazyadmin-cli -- search hermes --json`

## Milestone 2 - Shared Listener Table Projection

Opportunity: TUI and Web listener table behaviour is duplicated.

Relevant files:

- `crates/lazyadmin-tui/src/lib.rs`
- `crates/lazyadmin-web/static/app.js`
- `crates/lazyadmin-web/src/lib.rs`
- `crates/lazyadmin-runtime/src/view_model/mod.rs`
- `PLAN-16-listener-sort-hardening.md`

Problem:

Listener filters, sort semantics, owner/project labels, conflict/orphan/tracked
flags, warning counts, and row markers exist in separate TUI and Web
implementations. The Web UI currently computes some of these from raw snapshot
JSON in JavaScript. The TUI computes a related but different `RowVm` in Rust.

Solution:

Add a runtime listener table projection that both TUI and Web can render. The
Web can keep client-side URL state, but the row facts and accepted filter/sort
semantics should come from one module.

Candidate module:

- `crates/lazyadmin-runtime/src/view_model/listeners.rs`

Candidate concepts:

- `ListenerTableRow`
- `ListenerFilter`
- `ListenerSort`
- `ListenerSortColumn`
- `ListenerTableOptions`
- `build_listener_table(snapshot, options) -> ListenerTable`

Tasks:

- [ ] Define runtime listener row fields needed by both renderers:
  - [ ] Full listener ID.
  - [ ] Port.
  - [ ] Bind label.
  - [ ] Protocol.
  - [ ] Exposure.
  - [ ] Owner label.
  - [ ] Runtime or manager label.
  - [ ] Project label.
  - [ ] Confidence.
  - [ ] Warning count.
  - [ ] Conflict/orphan/tracked/project/system booleans.
  - [ ] Marker/signal classification as data, not terminal/Web styling.
- [ ] Move filter predicates into runtime.
- [ ] Move stable sort logic into runtime.
- [ ] Keep TUI-specific table columns in TUI, but feed them from runtime rows.
- [ ] Expose a Web endpoint only if needed:
  - Preferred: `GET /api/views/listeners?filter=&sort=&dir=&q=`
  - Acceptable first slice: `/api/snapshot` remains, but Web consumes a
    new embedded `listeners` projection from another read-only route.
- [ ] Remove duplicated Web owner/project/conflict logic after the endpoint or
  embedded projection exists.
- [ ] Update tests so TUI and Web use the same fixture expectations.

Validation:

- [ ] `cargo test -p lazyadmin-runtime`
- [ ] `cargo test -p lazyadmin-web`
- [ ] `cargo test -p lazyadmin-tui render_views`
- [ ] `cargo test -p lazyadmin-tui keybindings`
- [ ] `cargo run -p lazyadmin-cli -- tui --headless --json`
- [ ] Browser smoke for `lazyadmin web --port 0 --no-open` if a visible Web
  route changes.

## Milestone 3 - Single Live Snapshot Feed

Opportunity: long-lived snapshot refresh semantics are implemented in more than
one place.

Relevant files:

- `crates/lazyadmin-runtime/src/lib.rs`
- `crates/lazyadmin-cli/src/main.rs`
- `crates/lazyadmin-web/src/lib.rs`
- `docs/discovery-events-decision.md`

Problem:

Runtime already has `spawn_snapshot_refresh_task`, but the CLI has a separate
TUI refresh task and the Web server has its own refresh loop. Poll interval,
event debounce, event forwarding, cache age, and drop-counter semantics are
deep behaviour that should not be reimplemented by each surface.

Solution:

Deepen a runtime live-feed module that surfaces snapshots and discovery events
through one interface. TUI and Web become adapters over the same feed.

Candidate module:

- `crates/lazyadmin-runtime/src/live.rs`

Candidate interface:

```rust
pub struct LiveSnapshotFeed { /* internal task handles */ }

pub struct LiveSnapshotOptions {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub tick_ms: u64,
    pub event_debounce_ms: u64,
    pub event_channel_capacity: usize,
}

impl LiveSnapshotFeed {
    pub async fn spawn(options: LiveSnapshotOptions) -> anyhow::Result<Self>;
    pub fn snapshots(&self) -> watch::Receiver<Snapshot>;
    pub fn events(&self) -> broadcast::Receiver<DiscoveryEvent>;
    pub fn drops(&self) -> EventDropCounter;
}
```

The exact channel types can change during implementation. The important seam is
that event streams, polling, debounce, authoritative resnapshot, and drop
accounting live in runtime.

Tasks:

- [ ] Extract the current runtime refresh behaviour into `live.rs`.
- [ ] Preserve the discovery-events decision:
  - [ ] Events are hints.
  - [ ] Snapshot polling remains authoritative.
  - [ ] One long-lived drop counter feeds snapshot metadata and doctor data.
- [ ] Update TUI startup in `crates/lazyadmin-cli/src/main.rs` to use the
  runtime feed instead of `spawn_tui_refresh_task`.
- [ ] Update Web `AppState` to use the runtime feed instead of its local loop.
- [ ] Keep `lazyadmin events --once --json` behaviour stable.
- [ ] Add runtime tests for:
  - [ ] No event streams falls back to polling.
  - [ ] Event hints trigger a refresh.
  - [ ] Dropped event counts propagate to snapshots.

Validation:

- [ ] `cargo test -p lazyadmin-runtime`
- [ ] `cargo test -p lazyadmin-web`
- [ ] `cargo test -p lazyadmin-tui live_refresh`
- [ ] `cargo run -p lazyadmin-cli -- events --once --json`
- [ ] `cargo run -p lazyadmin-cli -- tui --headless --json`
- [ ] `cargo run -p lazyadmin-cli -- web --port 0 --no-open`

## Milestone 4 - TUI Interaction Core

Opportunity: TUI state transitions can be tested through a deeper interface.

Relevant files:

- `crates/lazyadmin-tui/src/lib.rs`
- `docs/tui.md`
- `docs/keybindings.md`
- `PLAN-24-global-search.md`

Problem:

`crates/lazyadmin-tui/src/lib.rs` is around 9.5k lines and owns app state,
command dispatch, input modes, search behaviour, selection preservation,
confirmation modals, rendering, themes, and tests. A mechanical file split would
be shallow. The deeper module is the interaction core: given current app state,
a command/key/event, and a snapshot/view model, decide the next state.

Solution:

Extract a TUI interaction module with an interface that tests and renderers can
share. Rendering should not need to know the rules for search origin view,
listener filter chips, selected listener identity restoration, modal
confirmation, or action-key dispatch.

Candidate module layout:

- `crates/lazyadmin-tui/src/interaction.rs`
- `crates/lazyadmin-tui/src/state.rs`
- `crates/lazyadmin-tui/src/render.rs`

Do not do all file moves in one PR. Start with interaction behaviour and keep
public `lazyadmin_tui` exports stable.

Tasks:

- [ ] Identify a narrow first reducer:
  - [ ] Listener sort commands preserve selection by listener ID.
  - [ ] Search `Esc`, `/`, and `Enter` transitions preserve the current
    PLAN-24 behaviour.
  - [ ] Modal confirmation consumes action keys before global commands.
- [ ] Move those transitions behind a function such as:

  ```rust
  pub fn apply_command(app: &mut App, command: Command, width: u16);
  ```

- [ ] Add tests that call the interaction interface directly instead of
  constructing terminal render output.
- [ ] Move rendering-only helpers only after command tests are stable.
- [ ] Keep headless TUI JSON stable unless a later issue intentionally changes
  `lazyadmin.tui.headless.v1`.

Validation:

- [ ] `cargo test -p lazyadmin-tui keybindings`
- [ ] `cargo test -p lazyadmin-tui render_views`
- [ ] `cargo test -p lazyadmin-tui live_refresh`
- [ ] `cargo run -p lazyadmin-cli -- tui --headless --json`

## Milestone 5 - Safe Action Planning Module

Opportunity: action safety should be reusable outside the CLI.

Relevant files:

- `crates/lazyadmin-cli/src/main.rs`
- `crates/lazyadmin-core/src/actions/mod.rs`
- `docs/action-safety.md`
- `docs/pause-restart-decision.md`
- GitHub issue #23 for inspector action preview dry-run output.

Problem:

The `free` command contains planning, confirmation copy, process-key guarded
execution, portless stop planning, resnapshot verification, and human/JSON
formatting in the CLI module. Core action types exist, but the safety behaviour
mostly sits outside the action module. This limits leverage for future TUI/Web
previews and makes action safety tests more coupled to CLI implementation.

Solution:

Deepen the action planning module so the CLI is an adapter over a reusable
planner/executor interface. Keep actual mutation conservative and preserve the
current direct-process and portless safety checks.

Candidate modules:

- `crates/lazyadmin-core/src/actions/free.rs`
- or `crates/lazyadmin-runtime/src/actions/free.rs` if execution needs runtime
  snapshot builders and adapter crates.

Decision needed before implementation:

- Put pure action plan construction in core.
- Put execution that needs adapters, resnapshotting, and OS signals in runtime
  or CLI.

Tasks:

- [ ] Split pure free-port planning from CLI formatting.
- [ ] Move the pure planner behind an action module interface.
- [ ] Keep process-key validation and resnapshot verification intact.
- [ ] Keep portless stop behaviour:
  - [ ] Route state remains read-only.
  - [ ] `free` stops the portless CLI process key, not descendant dev servers.
  - [ ] No automatic `portless prune`.
- [ ] Add unit tests for planner cases currently in CLI tests.
- [ ] Decide whether execution belongs in runtime after planner extraction.
- [ ] Coordinate with GitHub issue #23 so inspector command previews can call
  the planner rather than duplicating dry-run strings.

Validation:

- [ ] `cargo test -p lazyadmin-core`
- [ ] `cargo test -p lazyadmin-cli --features integration-portless free_portless_app`
- [ ] `cargo test -p lazyadmin-cli --test cli_smoke`
- [ ] `cargo run -p lazyadmin-cli -- free 65535 --dry-run --json`

## Milestone 6 - Documentation And Acceptance

Tasks:

- [ ] Update `docs/architecture.md` with the new runtime modules once they
  exist.
- [ ] Update `AGENTS.md` with durable caveats only:
  - [ ] Runtime relation lens ownership.
  - [ ] Listener table projection ownership.
  - [ ] Live snapshot feed ownership.
  - [ ] Any changed validation commands.
- [ ] Update `README.md` or `docs/tui.md` only when user-visible behaviour
  changes.
- [ ] Keep historical plans historical. Add back-references instead of editing
  old plans as if they never shipped.

Full validation before closing this plan:

```bash
cargo metadata --format-version=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p lazyadmin-cli --test cli_smoke
cargo test -p lazyadmin-runtime -p lazyadmin-web
cargo test -p lazyadmin-tui render_views
cargo test -p lazyadmin-tui live_refresh
cargo test -p lazyadmin-tui process_tree
cargo test -p lazyadmin-tui metrics
cargo test -p lazyadmin-tui theme
cargo test -p lazyadmin-tui keybindings
cargo run -p lazyadmin-cli -- --help
cargo run -p lazyadmin-cli -- export --json
cargo run -p lazyadmin-cli -- doctor --json
cargo run -p lazyadmin-cli -- overview --json
cargo run -p lazyadmin-cli -- search hermes --json
cargo run -p lazyadmin-cli -- events --once --json
cargo run -p lazyadmin-cli -- tui --headless --json
cargo run -p lazyadmin-cli -- web --port 0 --no-open
```

## GitHub Issue Breakdown

Published on approval:

- [#27](https://github.com/shuv1337/lazyadmin/issues/27) - Add a runtime snapshot relation lens
- [#28](https://github.com/shuv1337/lazyadmin/issues/28) - Consolidate live snapshot refresh into one runtime feed
- [#29](https://github.com/shuv1337/lazyadmin/issues/29) - Move free-port planning into a reusable action module
- [#30](https://github.com/shuv1337/lazyadmin/issues/30) - Move listener table facts into a shared runtime projection
- [#31](https://github.com/shuv1337/lazyadmin/issues/31) - Extract the TUI interaction core behind a testable interface
- [#32](https://github.com/shuv1337/lazyadmin/issues/32) - Update architecture docs and agent guidance after deepening

### Issue #27 - Add a runtime snapshot relation lens

Type: AFK

Blocked by: None.

What to build:

Create a runtime relation lens that centralizes repeated snapshot graph lookups
used by digest, search, inspector, and future listener projections.

Acceptance criteria:

- [ ] Runtime exposes a tested relation lens module for listener bind labels,
  owner labels, project labels, manager labels, owner PID lookup, and system
  listener classification.
- [ ] Digest, search, and inspector consume the relation lens for at least the
  duplicated listener facts.
- [ ] Public JSON contract tests remain stable.

### Issue #30 - Move listener table facts into a shared runtime projection

Type: AFK

Blocked by: #27.

What to build:

Create a runtime listener table projection used by TUI and Web so row facts,
filter predicates, warning counts, and sort semantics live behind one
interface.

Acceptance criteria:

- [ ] Runtime exposes tested listener table rows and filter/sort options.
- [ ] TUI listener rows render from the runtime projection.
- [ ] Web listener rows consume the same projection through a read-only route
  or equivalent shared payload.
- [ ] Existing TUI/Web sort and filter tests pass or are updated to assert the
  shared semantics.

### Issue #28 - Consolidate live snapshot refresh into one runtime feed

Type: AFK

Blocked by: None.

What to build:

Move long-lived polling, event-hint refresh, debounce, event broadcast, and drop
counter propagation into one runtime feed used by TUI and Web.

Acceptance criteria:

- [ ] TUI refresh uses the runtime feed.
- [ ] Web refresh uses the runtime feed.
- [ ] Discovery events remain hints and snapshot polling remains authoritative.
- [ ] Drop counts propagate through the same long-lived counter.
- [ ] Headless TUI, events, and Web smoke commands pass.

### Issue #31 - Extract the TUI interaction core behind a testable interface

Type: AFK

Blocked by: #30 for listener-selection pieces; otherwise can start with
search/modal transitions.

What to build:

Move TUI state transitions for search, command dispatch, listener selection,
sort changes, and confirmation modals behind a focused interaction interface.

Acceptance criteria:

- [ ] Search `Esc`, `/`, and `Enter` behaviour is tested through the
  interaction interface.
- [ ] Sort changes preserve selected listener identity through the interaction
  interface.
- [ ] Confirmation modal keys are consumed before global commands.
- [ ] Render tests continue to pass.

### Issue #29 - Move free-port planning into a reusable action module

Type: HITL for first design decision, then AFK implementation.

Blocked by: None. Coordinate with issue #23.

What to build:

Move pure `free` action planning out of the CLI and into a reusable action
module, while deciding where execution belongs.

Acceptance criteria:

- [ ] Pure free-port planner is callable outside the CLI.
- [ ] CLI `free` preserves current dry-run, confirmation, process-key, and
  resnapshot verification behaviour.
- [ ] Portless stop planning remains conservative and does not mutate route
  state.
- [ ] Planner tests move out of CLI-only coverage where practical.
- [ ] The design decision for planner-vs-executor ownership is documented.

### Issue #32 - Update architecture docs and agent guidance after deepening

Type: AFK

Blocked by: #27 through #31.

What to build:

Refresh architecture docs, AGENTS guidance, and validation commands after the
new runtime modules and action module exist.

Acceptance criteria:

- [ ] `docs/architecture.md` describes the new modules accurately.
- [ ] `AGENTS.md` records durable ownership/caveat changes.
- [ ] Historical plans receive back-references only where useful.
- [ ] Full workspace validation passes.
