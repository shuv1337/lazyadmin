# PLAN-16: Listener Sorting Hardening

Source review: [`lazyadmin-latest-commits-diff-review`](https://files.shuv.me/lazyadmin-latest-commits-diff-review.html) generated from `HEAD~3..HEAD` (`7bf3aef..4d8ac5d`).

This plan addresses every **Bad** and **Ugly** item from the diff review for the listener column sorting work:

- Web default sort is `port`, but the Web listener table does not render a Port column/header, so the active default is invisible and cannot be restored by clicking.
- No direct sorting tests were added.
- README/docs/CHANGELOG do not mention the new sort controls or keybinding actions.
- TUI and Web sort semantics diverge without documentation.
- Direction-reset behavior differs between TUI and Web.
- TUI selection is row-index based, so sorting can silently select a different listener.

## Goals

- Make listener sorting visible, predictable, and test-covered in both TUI and Web UI.
- Keep sorting presentation-only; discovery snapshots and JSON contracts remain unchanged.
- Preserve the read-only Web UI invariant: no new mutating routes, no server-side sort state.
- Avoid introducing a frontend framework or build step; Web remains vanilla JS/CSS embedded static assets.
- Update user-facing docs and changelog so the new controls are discoverable.

## Non-goals

- Do not change `lazyadmin.snapshot.v1` JSON schema.
- Do not add a new server endpoint solely for sorted listener rows unless a later plan moves all listener table projection into `lazyadmin-runtime`.
- Do not add Web runtime mutation actions.
- Do not redesign the listener table beyond the minimum needed to make sorting coherent.

## Relevant code references

| Area | File / symbol | Current issue |
|---|---|---|
| Config keybind actions | `crates/lazyadmin-core/src/config/mod.rs` — `KeybindAction::{SortNext, SortPrev, SortToggle}`, `ResolvedKeybindings::default_map()` | New controls exist but need docs + tests around resolved output. |
| TUI sort state | `crates/lazyadmin-tui/src/lib.rs` — `ListenerSortColumn`, `SortDirection`, `ListenerSort` | Sort columns are TUI-local and differ from Web columns. |
| TUI projection | `crates/lazyadmin-tui/src/lib.rs` — `build_view_model_with_state(...)`, `sort_listener_rows(...)` | Sorting is applied after filtering, but selected row remains index-based. |
| TUI command handling | `crates/lazyadmin-tui/src/lib.rs` — `handle_key(...)` `Command::{SortNext, SortPrev, SortToggle}` arms | Commands rebuild the view model without preserving selected listener identity. |
| TUI table header | `crates/lazyadmin-tui/src/lib.rs` — `listener_table_header(...)` | Header indicator works; tests should lock it down. |
| TUI help | `crates/lazyadmin-tui/src/lib.rs` — `help_lines(...)` | Help mentions sort keys; docs must match and keybinding overrides should remain reflected. |
| Web route state | `crates/lazyadmin-web/static/app.js` — `state.sortCol`, `state.sortDir`, `parseSortFromParams(...)`, `hashchange` handler | Default is `port` even though Port is not a visible column. |
| Web sorting | `crates/lazyadmin-web/static/app.js` — `sortListeners(...)`, `thLabel(...)`, `renderListeners(...)`, `attachSortHandlers(...)` | Sortable columns and direction reset semantics need correction/coverage. |
| Web styling | `crates/lazyadmin-web/static/app.css` — `.table th.sorted` | Active sort styling exists; may need accessible cursor/hover/ARIA polish. |
| Docs | `README.md`, `docs/tui.md`, `docs/keybindings.md`, `CHANGELOG.md` | Missing listener sort controls and action names. |
| Project handoff | `AGENTS.md` | Update only if the final implementation discovers durable caveats or changes validation commands. |

## Design decisions to make explicit before implementation

### Decision 1 — Web default sort column

Use one of these two approaches; prefer **Option A** unless product/design explicitly wants no Port column in the Web table.

| Option | Change | Pros | Cons |
|---|---|---|---|
| **A. Add a visible Port column to Web Listeners** | Render `Port` as the first sortable Web column and keep default `sort=port&dir=asc`. | Matches existing default; makes active sort visible; improves parity with TUI. | Adds another column to a dense table; needs responsive check. |
| B. Change Web default to `bind` | Keep current visible columns and make `bind` the default. | Minimal layout churn. | Loses direct port ordering in Web UI; diverges further from TUI; URL defaults change. |

Plan assumes Option A.

### Decision 2 — Direction reset semantics

Standardize both surfaces on: **when switching to a different sort column, reset direction to ascending**. Toggling direction should only happen when the active column is selected again or via TUI `SortToggle`.

Rationale:

- TUI already behaves this way in `ListenerSort::next_column(...)` and `ListenerSort::prev_column(...)`.
- Ascending default is easiest to predict for Port, Bind, Owner, Runtime/Scope/Exposure, Project, Confidence, and Warnings.
- It avoids stale URL state such as `#/listeners?dir=desc&sort=owner` after switching from a descending column.

### Decision 3 — Sort parity scope

Do **not** force exact column parity in this plan. Instead:

- TUI keeps terminal-fit columns: `Port`, `Bind`, `Owner`, `Runtime`, `Scope`.
- Web keeps richer browser columns: `Port`, `Bind`, `Exposure`, `Owner`, `Project`, `Confidence`, `Warnings`.
- Docs explicitly call out the difference.
- Tests ensure each surface's visible sortable headers match its accepted sort columns.

A later runtime-view-model plan can consolidate sorting if duplicate semantics become costly.

### Decision 4 — TUI selection preservation

When sorting changes, preserve selection by listener entity ID when possible:

1. Capture the selected row's `RowVm.id` before mutating `app.listener_sort`.
2. Rebuild the view model.
3. Find the new index of the same listener ID in the visible row set.
4. Set `selected_row` to that index.
5. If the old ID is no longer visible, clamp to the current behavior: `min(old_index, row_count - 1)`.

This should apply to `SortNext`, `SortPrev`, and `SortToggle`.

## Implementation phases

### Phase 0 — Establish baseline and fixtures

- [x] Confirm the current branch and diff scope before editing:

  ```bash
  git status --short
  git log --oneline --decorate -n 5
  ```

- [x] Run the focused tests currently known to pass so failures can be attributed to the implementation:

  ```bash
  cargo test -p lazyadmin-tui render_views
  cargo test -p lazyadmin-tui keybindings
  cargo test -p lazyadmin-web
  ```

- [x] Identify existing test helpers in `crates/lazyadmin-tui/src/lib.rs::tests` for constructing snapshots/listeners and interacting with `App`/`handle_key(...)`.
- [x] Identify existing Web static-asset tests in `crates/lazyadmin-web/src/lib.rs::tests` and decide whether JS behavior will be covered by:
  - [x] static source assertions in Rust tests, and/or
  - [x] a lightweight JS unit harness if one already exists.

Validation:

- [x] Baseline test commands above are green before behavior changes.

### Phase 1 — Fix Web default sort visibility

Preferred implementation: add a sortable `Port` column to the Web listener table.

- [x] Update `crates/lazyadmin-web/static/app.js` `renderListeners()` table header to include:

  ```js
  ${thLabel("port", "Port")}
  ```

  before `Bind`.

- [x] Update `listenerTableRow(l)` in `crates/lazyadmin-web/static/app.js` to render a matching Port cell.
  - [x] Display `l.port ?? "-"`.
  - [x] Keep bind/path display unchanged in the Bind column, even though TCP listeners will now show the port in both the compact Port column and the `addr:port` Bind value.
  - [x] Ensure Unix socket/path listeners without a port render gracefully.

- [x] Update listener empty-state table shape after adding the seventh column.
  - [x] `emptyRow(msg)` currently hardcodes `colspan="6"`; change it to accept a `colspan` parameter with a safe default, for example `emptyRow(msg, colspan = 6)`.
  - [x] Call `emptyRow("no listeners discovered yet", 7)` for the Listeners table.
  - [x] Preserve the existing 4-column empty states for Workloads and Processes by passing `4` there, or choose a table-local helper that keeps each colspan explicit.

- [x] Verify `parseSortFromParams(...)` valid column list includes exactly the columns rendered as sortable headers:

  ```js
  ["port", "bind", "exposure", "owner", "project", "confidence", "warnings"]
  ```

- [x] Update CSS only if needed for table density:
  - [x] `crates/lazyadmin-web/static/app.css` can add numeric alignment for the Port column if useful.
  - [x] Keep overflow behavior responsive; do not introduce a framework or build step.

- [x] Add accessible affordances while touching headers:
  - [x] Add `scope="col"` and `aria-sort="ascending|descending|none"` for active/inactive sortable headers.
  - [x] Make sorting keyboard-operable, not click-only. Prefer rendering a `<button type="button" class="sort-button" data-sort="...">` inside each `<th>` so Enter/Space behavior is native; alternatively add `tabindex="0"` plus Enter/Space key handling to the sortable header.
  - [x] Ensure cursor/hover/focus treatment communicates interactivity without relying only on color.

Validation:

- [x] In browser/manual smoke, `#/listeners` starts with Port header visibly marked ascending.
- [x] Clicking another header changes active marker to that header.
- [x] Clicking Port returns to Port sorting.
- [x] Unix socket/path rows do not break layout.
- [x] Empty listener state spans all visible columns.
- [x] Sort headers are reachable and operable from the keyboard.

### Phase 2 — Standardize Web direction reset semantics

- [x] Update `attachSortHandlers()` in `crates/lazyadmin-web/static/app.js` so a new column sets both params:

  ```js
  if (state.sortCol === col) {
    setParam("dir", state.sortDir === "asc" ? "desc" : "asc");
  } else {
    setParams({ sort: col, dir: "asc" });
  }
  ```

- [x] Extract a small pure sort-transition helper so behavior is testable without DOM/source-string assertions, for example:

  ```js
  function nextSortParams(currentSortCol, currentSortDir, clickedColumn) {
    if (currentSortCol === clickedColumn) {
      return { sort: clickedColumn, dir: currentSortDir === "asc" ? "desc" : "asc" };
    }
    return { sort: clickedColumn, dir: "asc" };
  }
  ```

- [x] If there is no helper for setting multiple params atomically, add a small route helper near `setParam(...)`, for example:

  ```js
  function setParams(values) {
    const r = state.route;
    Object.entries(values).forEach(([key, value]) => {
      if (value == null || value === "") r.params.delete(key);
      else r.params.set(key, value);
    });
    navigate(r.page, Object.fromEntries(r.params));
  }
  ```

- [x] Avoid double navigation when changing both `sort` and `dir`.
- [x] Preserve unrelated params such as `filter`, `q`, and `selected`.

Validation:

- [x] Starting from `#/listeners?sort=bind&dir=desc`, clicking Owner results in `#/listeners?sort=owner&dir=asc` while preserving existing unrelated params.
- [x] Clicking Owner again toggles `dir=desc`.
- [x] Browser back/forward still restores sort state.

### Phase 3 — Preserve TUI selection identity across sorting

- [x] Add a helper in `crates/lazyadmin-tui/src/lib.rs` to capture the selected listener ID from the currently visible listener rows.
  - Candidate placement: near existing `selected_row(app)` / listener row helpers, or near sort command handling.
  - The helper must respect the active listener filter and related-listener filter if selection applies to a filtered visible set.

- [x] Add a helper to restore selection by ID after `rebuild_view_model(app, width)`.
  - [x] Search the same visible row set for the captured ID.
  - [x] If found, update `app.selected_row` to the new index.
  - [x] If not found, clamp the previous index to the visible row count.

- [x] Update the `Command::SortNext`, `Command::SortPrev`, and `Command::SortToggle` arms in `handle_key(...)`:
  - [x] Capture `(selected_listener_id, old_selected_index)` before changing sort, but only for listener-like visible views where `visible_row_indices(app)` can identify listener rows (`Listeners`, `Public`, `Conflicts`, `Orphans`, and any related-listener scoped view).
  - [x] Mutate `app.listener_sort`.
  - [x] Rebuild view model.
  - [x] Restore selection by ID, or clamp to `min(old_index, row_count - 1)` when the ID is unavailable.
  - [x] Call `sync_row_selection(app)` after restoring or clamping so the inspector follows the newly selected row; `rebuild_view_model(app, width)` syncs once before the restore, so a second sync is required after changing `selected_row`.
  - [x] Keep the existing toast copy: `sorted by {label} {indicator}`.

- [x] Confirm no unrelated navigation commands accidentally get selection-preservation behavior.

Validation:

- [x] With at least three listeners, select a non-first listener, sort by another column, and verify the same listener remains selected if still visible.
- [x] If filter/sort changes make the old listener unavailable, selection clamps safely and does not panic.
- [x] Inspector content matches the restored/clamped selected listener after every sort command.
- [x] Existing render and keybinding tests remain green.

### Phase 4 — Add direct TUI sorting tests

Add tests in `crates/lazyadmin-tui/src/lib.rs::tests` using existing snapshot/listener test builders where possible.

- [x] Test `ListenerSort` column cycling:
  - [x] Default is `Port Asc`.
  - [x] `next_column()` cycles `Port → Bind → Owner → Runtime → Scope → Port` and resets `Asc`.
  - [x] `prev_column()` cycles backward and resets `Asc`.
  - [x] `toggle_direction()` preserves column and flips `Asc ↔ Desc`.

- [x] Test `sort_listener_rows(...)` ordering:
  - [x] Port sort orders numeric ports ascending and places `None` last for ascending.
  - [x] Descending reverses the ordering.
  - [x] Bind, Owner, Runtime, and Scope sorts are deterministic.

- [x] Test TUI header indicator:
  - [x] `listener_table_header(theme, ListenerSort { column: Bind, direction: Desc })` contains `Bind ▼` and not `Port ▲`.
  - [x] Keep this test resilient to Ratatui internals by inspecting cell text through available row/cell APIs or by extracting a small pure helper for header labels if needed.

- [x] Test command dispatch and selection preservation:
  - [x] Use `handle_key(...)` with default `]`, `[`, and `>` keybindings, or call command handling through the same resolved keybinding path existing tests use.
  - [x] Assert `app.listener_sort` changes as expected.
  - [x] Assert selected listener ID is preserved across sort change.

Validation:

```bash
cargo test -p lazyadmin-tui sort
cargo test -p lazyadmin-tui keybindings
cargo test -p lazyadmin-tui render_views
```

### Phase 5 — Add Web sorting tests / assertions

The Web UI is vanilla JS embedded via Rust. Prefer the smallest test layer that reliably prevents regression.

#### Minimum Rust static-asset assertions

Add or extend tests in `crates/lazyadmin-web/src/lib.rs::tests` to assert:

- [x] `app.js` renders a sortable Port header:

  ```text
  thLabel("port", "Port")
  ```

- [x] `parseSortFromParams(...)` valid columns include no sortable column that is absent from the rendered listener table.
- [x] `attachSortHandlers()` resets `dir` to `asc` when changing columns.
- [x] Sortable headers include accessible sort metadata and keyboard-operable controls (`<button>` or equivalent key handling).
- [x] Listener empty-state markup uses the correct colspan after adding the Port column.

#### Preferred behavior-level JS tests, if practical

If the repo already has, or can accept without heavy dependencies, a tiny JS test runner:

- [x] Extract pure helpers from `app.js` if needed:
  - [x] `parseSortFromParams(params)`
  - [x] `sortListeners(listeners, stateLike)` or keep state-bound and test via controlled state.
  - [x] `nextSortParams(currentSortCol, currentSortDir, clickedColumn)`
- [x] Test default parsing: no params → `{ sortCol: "port", sortDir: "asc" }`.
- [x] Test invalid column falls back to `port`.
- [x] Test invalid direction falls back to `asc`.
- [x] Test clicking current column toggles direction.
- [x] Test clicking/selecting a new column resets direction to `asc` and preserves unrelated params.
- [x] Test port sort places missing ports last in ascending order.

Validation:

```bash
cargo test -p lazyadmin-web
```

If adding a JS test command, document and run it here too.

### Phase 6 — Document the controls and semantics

Update user-facing docs after behavior is finalized.

- [x] `README.md`
  - [x] Update the keybinding list around the current TUI paragraph to include `[`, `]`, and `>`.
  - [x] Mention that the listener table supports column sorting.

- [x] `docs/tui.md`
  - [x] Add a short listener sorting subsection:
    - [x] `]` moves to the next sortable listener column.
    - [x] `[` moves to the previous sortable listener column.
    - [x] `>` toggles ascending/descending for the active sort column.
    - [x] Active sort is shown in the table header with `▲`/`▼`.
    - [x] Sort commands preserve selected listener identity where possible.

- [x] `docs/keybindings.md`
  - [x] Add action names:
    - [x] `sort_next`
    - [x] `sort_prev`
    - [x] `sort_toggle`
  - [x] Include an override example:

    ```toml
    [ui.keybindings.overrides]
    sort_next = "n"
    sort_prev = "N"
    sort_toggle = "."
    ```

  - [x] Confirm actual key parser syntax supports the documented examples. Current parser support is limited to known named keys, `ctrl+...`, or single-character specs; do not document `alt+...` unless parser and TUI normalization support are added in the same change.
  - [x] Add or update a core config test proving the documented `sort_next` / `sort_prev` / `sort_toggle` override example parses and appears in resolved keybindings.

- [x] Web UI docs, if present in `docs/tui.md` or `README.md` Web section:
  - [x] Mention clickable listener column headers.
  - [x] State that Web sort state is preserved in the URL hash.
  - [x] Document the intentional TUI/Web column difference if keeping it.

- [x] `CHANGELOG.md`
  - [x] Under `0.4.0`, add an entry summarizing listener table sorting in TUI/Web and new configurable keybind actions.

Validation:

- [x] `rg -n "sort_next|sort_prev|sort_toggle|listener.*sort|\] next|\[ prev|toggle direction" README.md docs/*.md CHANGELOG.md` finds the new documentation.
- [x] Documented key names match `KeybindAction::as_name()` in `crates/lazyadmin-core/src/config/mod.rs`.

### Phase 7 — Final validation and regression sweep

Run focused validation first:

```bash
cargo test -p lazyadmin-tui sort
cargo test -p lazyadmin-tui keybindings
cargo test -p lazyadmin-tui render_views
cargo test -p lazyadmin-web
```

Then run broader checks that are relevant to cross-crate public behavior:

```bash
cargo test -p lazyadmin-core keybindings
cargo run -p lazyadmin-cli -- config check --json
cargo run -p lazyadmin-cli -- tui --headless --json
timeout 5s cargo run -p lazyadmin-cli -- web --port 0 --no-open
```

Before handoff, run formatting/linting if code changed:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Full workspace validation when time allows:

```bash
cargo test --workspace
```

**Results:** All commands pass ✅
- 79 TUI tests pass (including 4 new sort tests)
- 18 Web tests pass (including 6 new static-asset assertions)
- 34 core tests pass (including 1 new config test)
- Full workspace test suite passes
- `cargo fmt --check` and `cargo clippy -D warnings` both clean

## Acceptance criteria

- [x] Web Listeners page shows the active default sort in a visible header.
- [x] Web users can click or keyboard-activate headers to return to the default sort column.
- [x] Web sort column changes reset direction to ascending and preserve unrelated hash params.
- [x] Web listener empty-state colspans remain correct after adding the Port column.
- [x] TUI sort commands keep the same listener selected when that listener remains visible, and the inspector matches the restored selection.
- [x] TUI sort state, row ordering, header indicator, and command dispatch have direct tests.
- [x] Web sort parsing/header/default behavior has at least static regression assertions; behavior-level tests exist if practical.
- [x] README, `docs/tui.md`, `docs/keybindings.md`, and `CHANGELOG.md` mention the new controls.
- [x] Targeted tests and validation commands pass.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Adding a Web Port column makes the table too dense on narrow screens. | Verify responsive overflow; the existing `.table-wrap` should allow horizontal scrolling. Keep Port cell compact and numeric. Also update listener empty-state colspan when the column count changes. |
| TUI selection preservation accidentally ignores active listener filters. | Implement capture/restore against the visible row set, not raw `view_model.rows` alone, and add tests for filtered listener views where practical. Re-run `sync_row_selection(app)` after restoring so the inspector matches the selected row. |
| Ratatui row/cell internals make header tests brittle. | Extract a pure `listener_header_labels(sort) -> Vec<String>` helper and test that instead, while rendering uses the helper. |
| JS behavior tests require tooling not currently present. | Start with Rust static-asset assertions; only add a JS test harness if it is lightweight and aligned with repo conventions. |
| Docs document unsupported key syntax. | Use only supported specs (known names, `ctrl+...`, or single characters) unless parser/TUI support expands; verify examples with `lazyadmin config check --json` or keybinding parser tests before finalizing docs. |

## Follow-up candidates outside this plan

- Move listener table sorting/filtering into `lazyadmin-runtime::view_model` so TUI and Web can share column definitions and ordering semantics.
- Add Web end-to-end smoke screenshots for sorted listener tables once browser-based UI testing is available in the repo.
- Add a user preference for default listener sort column/direction if users ask for persistent sort choices.
