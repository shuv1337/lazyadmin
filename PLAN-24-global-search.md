# PLAN-24 — Global fuzzy search (default focus on launch)

> Parent issues: #13 (UX overhaul), #22 (filter affordance / fuzzy hint).
> Adds a new tracking-worthy capability that is not in #22 today: cross-entity global search across listeners + processes that is the **default keyboard focus** when the TUI launches, with port-number shortcuts.
> Open a new issue from this plan; thread it under #13.

## Goal (one line)

Launch `lazyadmin` and immediately start typing — `hermes` filters to all listeners/processes whose name or path matches, `5432` filters to listeners bound to port 5432 (with prefix fallback) and the process with PID 5432 if present. No keystroke required to "enter search mode."

## Operator scenarios

1. `lazyadmin` → type `hermes` → see Hermes-related listeners (e.g. `:7777 hermes-node`) and Hermes processes (`/usr/local/bin/hermes …`) grouped under one screen → Enter on the top result opens the inspector for it.
2. `lazyadmin` → type `5432` → see exactly the listener(s) on `:5432` plus process PID 5432 if present → Enter inspects.
3. `lazyadmin` → type `54` → see exact `:54` listeners if present; otherwise fall back to prefix match and surface `:5432`, `:5400`, `:5410`, etc. with a `(prefix)` hint.
4. User hits `Esc` while typing → query clears, focus blurs to the results pane, and the body returns to the previous view (Overview digest on launch). `q` quits from normal/nav mode.
5. User wants search again → presses `/` → focus returns to the input with whatever query is currently stored in `app.query` (empty after Esc-clear; preserved after Enter-open).

## Design decisions (locked for implementation)

| Concern | Decision |
|---|---|
| Landing screen | Keep the existing `ViewKind::Overview` digest as the default `active_view`. Search bar is **auto-focused on launch** (`InputMode::Search`). Empty query renders the current `active_view` body, usually Overview. First typed character switches body to transient `ViewKind::Search`. |
| Search scope | **Listeners** (port + bind/path + protocol + exposure + owner label + workload display names + project name when available) and **Processes** (pid + user + exe + cwd + cmdline). Workloads/projects/managers/doctor are explicitly **not** independent result groups in this round; they only enrich listener/process haystacks. |
| Query classification | Empty → `empty`. Pure digits that fit `u16` → `port`. Pure digits that do not fit `u16` but fit positive `i32` → `pid`. Everything else (including `-1`) → `text`. |
| Port match rule | Exact listener port first; if 0 listener hits, fall back to **prefix** (`s.starts_with(query)`) on the port's decimal string. Port queries also surface a process whose `pid == port` if present. |
| PID match rule | PID queries surface process `pid == query`; they also run normal text fuzzy matching so a PID-like string in cmdline/exe can still match. PID queries never prefix-match listener ports. |
| Esc | In `InputMode::Search`, `Esc` clears query, sets `mode = Normal`, focuses `Pane::Rows`, and restores `return_view_on_clear.unwrap_or(search_origin_view)` if the active view is Search. On launch, `search_origin_view` is Overview. It does not quit. |
| Re-focus | `/` (and any configured filter/toggle-filter binding) sets `mode = Search` without clearing `app.query`. Plain letters in nav mode keep existing single-letter bindings (`a/p/c/o/u/t/S/etc`) — search is never silently triggered after blur. |
| Results layout | Grouped sections with counts: `Listeners (returned/total)`, `Processes (returned/total)`. Top row of the first non-empty group is highlighted on first paint. |
| Input position | Single line at the very top of the screen, **above** the existing header. The header (`lazyadmin  Overview  N listeners  …`) drops one row down. |
| Enter behavior | Enter on a highlighted result opens the inspector for that entity and **switches `active_view`** to that entity's natural view (`Listeners` or `Processes`), preserving the query so the per-view filter is the same set. |
| Strategy hint | Right-aligned next to the input: `text query` / `port :5432` / `port :54 (prefix)` / `pid 12345`. Empty query has an empty `strategy_hint` in JSON and a placeholder in UI. |
| Persist query | `app.query` remains the single source of truth. Search input writes into it; existing per-view fuzzy filters read from it. Esc-clear empties it; Enter-open preserves it. |
| Palette parity | `:search <q>` sets `app.query = q`, `app.mode = InputMode::Search`, `app.return_view_on_clear = Some(previous_view)`, switches to `ViewKind::Search`, and rebuilds. `:search` with no query just focuses the input. |
| CLI parity | `lazyadmin search <query> [--json] [--limit N]` lists matched listeners + processes. Reuses the same matcher and JSON contract. |
| Web parity | New `GET /api/search?q=…&limit=…` returning the same grouped shape; persistent top search bar in `crates/lazyadmin-web/static/index.html`. `/` focuses the global search; `Ctrl/Cmd+K` opens the existing palette. |
| Rail behavior | `ViewKind::Search` is transient and **not** added to `RAIL_ENTRIES`. While Search is active, keep Overview highlighted if the search was launched from Overview; otherwise keep the previous canonical rail item highlighted. |
| System-service hiding | TUI search respects `app.show_system` exactly like the listener table. CLI/Web search the complete cached/fresh snapshot by default because their current read-only views are not governed by the TUI system-row toggle. |

## Architecture / shared projection

Put the matcher in `crates/lazyadmin-runtime/src/view_model/search.rs` so TUI, CLI, and Web reuse one implementation (mirrors the digest / inspector / doctor_groups / header_pip pattern called out in `AGENTS.md`).

```text
crates/lazyadmin-runtime/src/view_model/search.rs   ← new module
  pub const SEARCH_SCHEMA_VERSION: &str = "lazyadmin.search.v1";
  pub const DEFAULT_SEARCH_LIMIT: usize = 200;
  pub const MAX_SEARCH_LIMIT: usize = 500;

  pub struct SearchOptions { limit: usize, show_system: bool }
  pub struct SearchQuery { raw: String, normalized: String, kind: SearchKind }
  pub enum SearchKind { Empty, Text { text: String }, Port { port: u16 }, Pid { pid: i32 } }
  pub struct SearchResults {
      schema_version: String,
      query: SearchQuery,
      listeners: SearchGroup<ListenerHit>,
      processes: SearchGroup<ProcessHit>,
      strategy_hint: String,
      fell_back_to_prefix: bool,
      elapsed_ms: u128,
  }
  pub struct SearchGroup<T> {
      total: usize,       // matches before limit
      returned: usize,    // hits.len()
      truncated: bool,
      hits: Vec<T>,
  }
  pub struct ListenerHit {
      id: ListenerId,
      port: Option<u16>,
      bind: String,
      protocol: Protocol,
      exposure: Exposure,
      owner_label: String,
      workload_labels: Vec<String>,
      project_label: Option<String>,
      score: i64,
      matched_indices: Vec<usize>,
      is_system: bool,
  }
  pub struct ProcessHit {
      key: ProcessKey,
      pid: i32,
      user: Option<String>,
      exe_or_argv0: String,
      cmdline_compact: String,
      cwd: Option<PathBuf>,
      score: i64,
      matched_indices: Vec<usize>,
      is_system: bool,
  }
  pub fn run(snapshot: &Snapshot, query: &str, options: SearchOptions) -> SearchResults
```

Implementation notes:

- Reuse `fuzzy_matcher::skim::SkimMatcherV2` in `lazyadmin-runtime`.
- Do **not** drop `fuzzy-matcher` from `lazyadmin-tui` until every TUI palette/filter use has been replaced. Current TUI palette filtering still uses `SkimMatcherV2`; either keep the dependency in both crates or add a small runtime helper and update all TUI call sites.
- `matched_indices` is a `Vec<usize>` of matched character indices from `fuzzy_indices()`. Do not call this `matched_ranges` unless implementation explicitly coalesces indices into UTF-8-safe ranges.
- Clamp `SearchOptions.limit` to `1..=MAX_SEARCH_LIMIT` inside runtime so CLI/Web/TUI cannot accidentally render thousands of rows.
- Filter process hits with `pid == 0` out of search results to avoid portless alias noise. Listener hits remain unaffected.
- Runtime search should emit a tracing span/event without logging raw query by default: include `query_kind`, `normalized_len`, `limit`, `show_system`, `listener_total`, `process_total`, `fell_back_to_prefix`, and `elapsed_ms`.

### Query classifier rules

```rust
fn classify(raw: &str) -> SearchKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return SearchKind::Empty;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(port) = trimmed.parse::<u16>() {
            return SearchKind::Port { port };
        }
        if let Ok(pid) = trimmed.parse::<i32>() {
            if pid > 0 {
                return SearchKind::Pid { pid };
            }
        }
    }
    SearchKind::Text { text: trimmed.into() }
}
```

Table-driven classifier tests must cover: `""`, `"hermes"`, `"5432"`, `"54"`, `"-1"`, `"99999"`, and an integer overflowing `i32`.

### Matching rules

- `SearchKind::Empty`: return empty groups, `strategy_hint = ""`, `fell_back_to_prefix = false`.
- `SearchKind::Port { port }`:
  - listeners: exact `l.port == Some(port)` first.
  - if exact listener hits are zero, prefix-match TCP/UDP listener port strings using the normalized raw query and set `fell_back_to_prefix = true`.
  - processes: exact `process.pid == port as i32` only, excluding pid 0.
  - hint: `port :<raw>` or `port :<raw> (prefix)`.
- `SearchKind::Pid { pid }`:
  - processes: exact PID first, plus fuzzy text matches over process haystack.
  - listeners: text fuzzy matching only if listener haystack contains the PID-like string via owner/workload/project enrichment; no listener port prefix fallback.
  - hint: `pid <pid>`.
- `SearchKind::Text { text }`:
  - fuzzy match listener haystack from `listener_search_text(listener, snapshot)`.
  - fuzzy match process haystack from `process_search_text(process)`.
  - hint: `text query`.
- Ranking: score descending, then listener `port ASC NULLS LAST`, listener `bind ASC`, process `pid ASC`, stable original snapshot order as final tie-breaker.

### Public JSON contract

`lazyadmin search <query> --json`, `/api/search`, and headless TUI `view_model.search` all serialize the same additive shape:

```json
{
  "schema_version": "lazyadmin.search.v1",
  "query": {
    "raw": "5432",
    "normalized": "5432",
    "kind": { "type": "port", "port": 5432 }
  },
  "listeners": {
    "total": 1,
    "returned": 1,
    "truncated": false,
    "hits": [
      {
        "id": "tcp:127.0.0.1:5432:123",
        "port": 5432,
        "bind": "127.0.0.1:5432",
        "protocol": "tcp",
        "exposure": "loopback",
        "owner_label": "postgres",
        "workload_labels": [],
        "project_label": null,
        "score": 10000,
        "matched_indices": [0, 1, 2, 3],
        "is_system": false
      }
    ]
  },
  "processes": {
    "total": 0,
    "returned": 0,
    "truncated": false,
    "hits": []
  },
  "strategy_hint": "port :5432",
  "fell_back_to_prefix": false,
  "elapsed_ms": 0
}
```

Serde requirements:

- Use `#[serde(rename_all = "snake_case")]` for enums and fields where applicable.
- Use an internally tagged shape for `SearchKind`, e.g. `#[serde(tag = "type", rename_all = "snake_case")]`.
- Add `Default` for `SearchResults` that returns `schema_version`, empty query/groups, empty hint, and zero elapsed time so `ViewModel::default()` remains cheap.

## TUI changes

### New `InputMode` and transient `ViewKind`

- Add `InputMode::Search`.
- Add `ViewKind::Search`, but keep `ViewKind::default() == ViewKind::Overview`.
- Fold the old `/` Filter UX into global Search:
  - Keep config action names `filter` / `toggle_filter` for backward compatibility.
  - Map them to a search-focus command.
  - Remove or stop using `InputMode::Filter` once all tests are updated. If keeping the enum temporarily lowers churn, ensure no footer-only `Filter:` prompt remains.
- Startup: keep `active_view = initial_view.unwrap_or(ViewKind::Overview)` and set `mode = InputMode::Search` after `App` construction.

### App/ViewModel state additions

```rust
pub struct App {
    // existing fields…
    pub mode: InputMode,
    pub query: String,
    pub return_view_on_clear: Option<ViewKind>,
    pub search_origin_view: ViewKind, // for rail highlighting while Search is active
}

pub struct ViewModel {
    // existing fields…
    pub search: SearchResults,
}
```

Build `ViewModel.search` in `build_view_model_with_state` by delegating to:

```rust
lazyadmin_runtime::view_model::search::run(
    snapshot,
    filter,
    SearchOptions { limit: DEFAULT_SEARCH_LIMIT, show_system },
)
```

Important: the existing per-view fuzzy filtering currently filters **all** summary/workload/doctor rows when `filter` is non-empty. Global Search scope is only listeners/processes. Decide in implementation whether `app.query` should continue filtering non-search views; if yes, document that as existing behavior. If no, change only Listeners/Processes to consume `app.query` and leave Doctor/Workloads/etc unfiltered.

### Render order (with search bar at top)

Current `render_view_kind` splits vertically as `[header(3), body, footer(1)]`. Change to:

```text
[search_bar(1)] [header(3)] [body] [footer(1)]
```

Implementation detail: `render_view_kind` currently receives only `ViewModel` and `RenderContext`, not `App`. Extend `RenderContext` with the necessary search/input data:

```rust
struct RenderContext<'a> {
    // existing fields…
    query: &'a str,
    input_mode: InputMode,
    search_origin_view: ViewKind,
}
```

Then implement `render_search_bar(view_model, frame, area, theme, ctx)`:

- Left: `🔎 ` (or ASCII `> ` for narrow/non-emoji fallback) + query text + caret when `ctx.input_mode == InputMode::Search`.
- Right: strategy hint and `N matched` once a query exists (`listeners.total + processes.total`).
- Dimmed placeholder when not focused: `Press / to search listeners + processes`.
- Render even on narrow/refusal screens so the affordance is stable; then render refusal/body below it.
- Palette footer remains for `InputMode::Palette`; the old `Filter:` footer at current `crates/lazyadmin-tui/src/lib.rs:5782` is removed.

### Search results view

New `render_search_view(view_model, frame, area, theme, ctx)`:

- Two stacked sections from `view_model.search`:
  - `Listeners (returned/total)` table: port, bind, owner, exposure.
  - `Processes (returned/total)` table: pid, user, exe, cmdline-compact.
- Show `… +N more` for a group when `truncated == true` using `total - returned`.
- Selection model: `app.selected_row` indexes the flat concatenation of listener hits followed by process hits.
- Add helpers:

```rust
enum SearchHitRef<'a> { Listener(&'a ListenerHit), Process(&'a ProcessHit) }
fn search_hit_count(results: &SearchResults) -> usize;
fn search_hit_at(results: &SearchResults, flat_index: usize) -> Option<SearchHitRef<'_>>;
```

### TUI state-machine integration checklist

Add explicit Search handling in all of these existing paths:

- `parse_view_kind` accepts `search` for palette/hidden programmatic entry.
- `title_for_view(ViewKind::Search) -> "Search"`.
- `canonical_rail_view(ViewKind::Search)` returns `search_origin_view` or `Overview` so Search is not added to `RAIL_ENTRIES`.
- `cli_hints_for_view(ViewKind::Search)` includes `lazyadmin search <query> --json`.
- `render_main_pane` dispatches to `render_search_view`.
- `visible_row_indices` does not use listener rows for Search.
- `scroll_rows`, `Home`, and `End` use `search_hit_count` when active view is Search.
- `sync_row_selection` clamps to `search_hit_count` and updates inspector:
  - listener hit → reuse `inspector_for_row` by finding the matching `RowVm`/listener, or call `InspectorView::lookup` directly.
  - process hit → reuse `inspector_for_process`.
- `selected_row(app)` remains listener-only; add a separate `selected_search_hit(app)` for Search so runtime actions do not accidentally target the wrong listener.
- `active_toast_message` treats `InputMode::Search` as input-active like Palette/Filter.
- Tests that construct `RenderContext` need the new fields.

### Key handling

In `handle_key`:

1. If `mode == InputMode::Search`:
   - `Esc` → clear query, `mode = Normal`, `pane = Pane::Rows`, restore `app.return_view_on_clear.unwrap_or(app.search_origin_view)` when `active_view == Search`, rebuild, status `search cleared`.
   - `Enter` → if a result is highlighted, `jump_to_search_result(app, result, width)`.
   - `Backspace` → mutate `app.query`; if query becomes empty, restore `return_view_on_clear.unwrap_or(search_origin_view)` when active view is Search; rebuild.
   - `Char(c)` → if query was empty and `active_view != Search`, set `return_view_on_clear = Some(active_view)`, `search_origin_view = active_view`, then set `active_view = Search`; append char; rebuild.
   - `Tab` / `Shift+Tab` → set `mode = Normal`, cycle pane.
   - `Up` / `Down` / `PageUp` / `PageDown` / `Home` / `End` → scroll result selection; do not mutate query and do not blur unless implementation decides arrow navigation is row focus.
2. If `mode == InputMode::Normal` and key maps to filter/toggle-filter or raw `/` → `mode = Search`, preserve `app.query`, set `return_view_on_clear` only if currently not Search.
3. Other modes unchanged. While `InputMode::Search`, `q` is a literal character, matching the existing Filter precedent.

### Jump behavior

`jump_to_search_result(app, result, width)`:

- Listener hit:
  - `app.listener_filter = ListenerFilter::All`
  - `app.related_listener_filter = None`
  - `set_active_view(app, ViewKind::Listeners, width)`
  - find/select matching listener id after rebuild
  - preserve `app.query`
- Process hit:
  - set `app.selected_process = Some(hit.key.clone())`
  - `set_active_view(app, ViewKind::Processes, width)`
  - find/select matching process row after rebuild if possible
  - preserve `app.query`
- Emit tracing event `tui.search.open_result` with `kind`, `pid`/listener id hash or length-safe identifier, and query kind.

### Footer hint copy

Update `footer_hint_line` (`crates/lazyadmin-tui/src/lib.rs:3063`) to replace `[/] filter` with `[/] search   [enter] open   [esc] clear   [tab] pane` in Rows. Add a Search-specific branch:

```text
[/] focus search   [enter] open result   [esc] clear   [tab] pane   [q] quit
```

### Help overlay

Add a "Global search" section to `help_lines()`:

- `/` focus search input
- type text or a port number
- Enter opens highlighted result
- Esc clears + blurs
- Tab moves focus to rows/inspector
- `Ctrl/Cmd+K` in Web opens palette; TUI palette remains `:`

### Theme

No new palette tokens required — reuse `theme.accent` for input caret, `theme.footer` for placeholder/strategy hint, `theme.selection` for highlighted result. If matched-character highlighting is implemented later, use existing semantic colors and the `matched_indices` data.

## CLI changes

New subcommand in `crates/lazyadmin-cli/src/main.rs`:

```text
lazyadmin search <query> [--json] [--limit N]
```

Implementation:

- Add `Search(SearchArgs)` to the Clap `Command` enum.
- `SearchArgs { query: String, limit: Option<usize> }`; use global `--json` rather than a subcommand-local `--json` unless Clap compatibility requires both.
- Build a fresh snapshot via existing `build_snapshot`.
- Call `lazyadmin_runtime::view_model::search::run(&snapshot, &query, SearchOptions { limit, show_system: true })`.
- Human output prints two compact tables and truncation footers.
- JSON output serializes `SearchResults` directly.
- Add tracing span `cli.search` with query kind/length, limit, total hits, elapsed time.
- Add `lazyadmin search --help` coverage through Clap snapshot/help tests if present; otherwise add a targeted unit/integration test.

## Web changes

### API

In `crates/lazyadmin-web/src/lib.rs`:

- New `GET /api/search?q=<query>&limit=<N>` route returning `SearchResults` JSON.
- Query extractor:

```rust
struct SearchQueryParams { q: Option<String>, limit: Option<usize> }
```

- `limit = min(limit.unwrap_or(DEFAULT_SEARCH_LIMIT), MAX_SEARCH_LIMIT)`.
- Use cached snapshot via `state.snapshot().await`.
- Call runtime matcher with `show_system: true`.
- Add tracing span/event with query kind/length, limit, total hits, and elapsed time.
- Route remains under existing local-origin guard.

### Static UI

Current Web UI already uses `/` to open the palette and per-view `q` filters. Change deliberately:

- Add persistent top-level search markup near the topbar:

```html
<div class="global-search-shell">
  <label for="global-search">Search</label>
  <input id="global-search" type="search" autocomplete="off" placeholder="search listeners or processes (try 5432 or hermes)">
  <span id="global-search-hint"></span>
</div>
```

- `/` focuses `#global-search` when the target is not a text input.
- Existing palette remains available via `Ctrl/Cmd+K` and the palette button. Rename the button copy from `/ search` to `⌘K palette` or `Ctrl+K palette`.
- Keep Web search URL-state out of scope for v1. Store global search input in JS state, not hash `q`, to avoid colliding with existing per-view filter params.
- Decide whether to keep per-view `searchToolbar()`:
  - Recommended v1: remove per-view toolbar from Listeners/Workloads/Processes after global search lands, or rename its state away from `filterText`/`q` if keeping it temporarily.
  - If kept, document that global search is cross-entity while per-view filters are page-local.
- Debounce input (~120ms), fetch `/api/search?q=…`, render result groups under `#page` while query is non-empty, and restore the current route page when query is empty.
- Auto-focus the input on `DOMContentLoaded`.
- Clicking a listener/process search row opens the existing inspector and navigates to its natural page.
- No frameworks, no inline JS — keep behavior in `crates/lazyadmin-web/static/app.js` and styles in `app.css`.

## Schema / docs

- Add `docs/schema/search-v1.md` (not `search.json`, unless adding real JSON Schema files project-wide) with examples for empty, text, port exact, port prefix, and PID queries.
- Update `docs/keybindings.md` for Search mode, `/` re-focus, Esc behavior, and palette distinction.
- Update `docs/tui.md` to say `lazyadmin` starts with global search focused while rendering Overview until a query is typed.
- Update `README.md` Quickstart to call out "type to filter immediately" and `lazyadmin search <query> --json`.
- Update `AGENTS.md`:
  - Add `lazyadmin_runtime::view_model::search` to the shared projection layer bullet.
  - Add `/api/search` to the Web UI API surface bullet.
  - Add `lazyadmin search` to the validation commands block.
  - Note the launch-default `InputMode::Search` caveat alongside existing TUI live-refresh notes.

## Validation criteria

A change is "done" when:

- `lazyadmin tui --headless --json` includes `view_model.search` with schema version `lazyadmin.search.v1`, empty listener/process groups, and `strategy_hint: ""` for empty query.
- A unit-testable `init_app`/constructor path proves launch state has `active_view == Overview`, `mode == Search`, and empty `query`.
- Running the TUI interactively and immediately typing `hermes` produces a Search view with Hermes-named listeners + processes (verify locally with `lazyadmin tui`; the dogfood-tui skill is acceptable for automated capture).
- Running the TUI interactively and immediately typing `5432` produces listeners on `:5432` plus PID 5432 if present. Typing `54` produces prefix fallback only when no exact `:54` listener exists.
- `Esc` clears the input, blurs to rows, and restores the previous view/digest. `/` re-focuses. `q` quits only when not typing.
- `lazyadmin search hermes --json` returns the documented `lazyadmin.search.v1` shape.
- `curl localhost:<port>/api/search?q=hermes` returns the same JSON shape as the CLI.
- Telemetry spans/events exist for runtime matcher, CLI search, Web search route, TUI search input/result open, and include latency + hit counts without raw query leakage by default.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass.

## New tests

Runtime:

- `crates/lazyadmin-runtime/src/view_model/tests.rs` — classifier tests for `""`, `"hermes"`, `"5432"`, `"54"`, `"-1"`, `"99999"`, and overflowing integer.
- `crates/lazyadmin-runtime/src/view_model/search.rs` tests — synthetic snapshot fixtures asserting exact port, prefix fallback, PID hit, text fuzzy listener/process hits, pid 0 exclusion, system filtering, truncation metadata, and stable ordering.
- JSON contract test for the exact `SearchResults` shape.

TUI:

- Render test at 120 cols asserting search bar renders above header.
- Render tests at 120/90/70 columns asserting Search view renders both groups and truncation footer.
- Constructor/init test asserting default launch focus (`mode == Search`) without entering raw terminal mode.
- Key handling tests for first character switching Overview → Search, Esc restore, `/` re-focus preserving query, and Enter jump preserving query.
- Regression test that listener single-letter filters still work after Search blur.

CLI:

- Fixture/synthetic test for `lazyadmin search hermes --json` or a direct `run_search_for_snapshot` helper if live snapshots make CLI integration nondeterministic.
- Empty query returns `kind.type == "empty"`, empty groups, and `strategy_hint == ""`.

Web:

- Route test for `/api/search?q=5432` returning `schema_version == "lazyadmin.search.v1"`.
- Route test clamps `limit` to `MAX_SEARCH_LIMIT`.
- Static tests ensuring no inline JS is introduced and `/api/search` is referenced from `app.js`.

## Implementation tasks (in order)

### Phase 0 — Contract + telemetry decisions

- [ ] Finalize the exact `SearchResults` serde shape from this plan before coding UI renderers.
- [ ] Add telemetry requirements to implementation notes/tests: runtime matcher, CLI search, Web route, TUI input/result-open.
- [ ] Decide whether non-search TUI views continue to consume `app.query` for Workloads/Doctor/etc. Recommendation: preserve current behavior for now, but only global Search result groups are listeners/processes.

### Phase 1 — Runtime matcher

- [ ] Add `fuzzy-matcher` to `crates/lazyadmin-runtime/Cargo.toml`; keep it in TUI until palette/filter call sites are moved or replaced.
- [ ] Create `crates/lazyadmin-runtime/src/view_model/search.rs` with `SearchOptions`, `SearchQuery`, `SearchKind`, `SearchGroup`, `SearchResults`, `ListenerHit`, `ProcessHit`, `run(...)`, `listener_search_text(...)`, and `process_search_text(...)`.
- [ ] Implement classifier and matching rules above.
- [ ] Implement `show_system` filtering for TUI callers using the same predicate as current TUI system-row hiding.
- [ ] Add truncation metadata and stable ordering.
- [ ] Add `SearchResults` + sub-types to `view_model/mod.rs` re-exports and `RuntimeViewModels` if useful.
- [ ] Add runtime tracing around matcher execution.
- [ ] Add runtime tests.

### Phase 2 — TUI default-focus + search view

- [ ] Add `InputMode::Search` and `ViewKind::Search` variants.
- [ ] Add `return_view_on_clear` and `search_origin_view` to `App` and defaults.
- [ ] Extract an `init_app`/constructor helper so launch state can be tested without running terminal raw mode.
- [ ] Set `app.mode = InputMode::Search` at launch after constructing App.
- [ ] Extend `ViewModel` with `pub search: SearchResults` and populate inside `build_view_model_with_state`.
- [ ] Extend `RenderContext` with query/input-mode/search-origin fields.
- [ ] Implement `render_search_bar(...)` and prepend it to vertical layout.
- [ ] Implement `render_search_view(...)` with two grouped tables, flat selection, empty states, and truncation footers.
- [ ] Add explicit Search handling to `parse_view_kind`, `title_for_view`, `canonical_rail_view`, `cli_hints_for_view`, `render_main_pane`, `visible_row_indices`, `sync_row_selection`, `scroll_rows`, Home/End, and toast/input-active logic.
- [ ] Update `handle_key` for `InputMode::Search` (Esc, Enter, Backspace, Char, Tab/Shift+Tab, Arrow/PgUp/PgDn/Home/End).
- [ ] Update Normal-mode `/` and configured filter/toggle-filter bindings to focus Search and not clear the query.
- [ ] Implement `jump_to_search_result` for listener and process hits.
- [ ] Add `:search <q>` palette command.
- [ ] Remove the old `Filter:` footer rendering; palette footer stays.
- [ ] Update `footer_hint_line` copy and `help_lines`.
- [ ] Add TUI telemetry for search focus/input/result-open.
- [ ] Add TUI render/key/state tests.

### Phase 3 — CLI

- [ ] Add `Search(SearchArgs)` to the Clap command enum.
- [ ] Wire it to call `lazyadmin_runtime::view_model::search::run` against a fresh snapshot.
- [ ] Human formatter prints two compact tables and truncation footers.
- [ ] `--json` serializes `SearchResults`.
- [ ] Add CLI tracing span around search.
- [ ] Add tests.

### Phase 4 — Web

- [ ] Add `GET /api/search?q=…&limit=…` route in `crates/lazyadmin-web/src/lib.rs`.
- [ ] Add query extractor / handler that calls the runtime matcher against the cached snapshot with clamped limit.
- [ ] Add route tracing.
- [ ] Add route tests.
- [ ] Add persistent global search input to `index.html`.
- [ ] Change `/` to focus global search; keep palette on `Ctrl/Cmd+K` and update button copy.
- [ ] In `app.js`, add debounced fetch, request cancellation/stale-response guard, results renderer, row click → inspector/page navigation, and auto-focus on load.
- [ ] Resolve conflict with existing per-view `searchToolbar()` and `filterText`/hash `q` usage by removing it or clearly renaming/separating it.
- [ ] In `app.css`, style search bar + results sections consistent with Night Owl palette.
- [ ] Add static no-inline-JS tests if not already covered.

### Phase 5 — Schema / docs

- [ ] Add `docs/schema/search-v1.md` with JSON examples and field descriptions.
- [ ] Document new keybindings in `docs/keybindings.md`.
- [ ] Update `docs/tui.md`.
- [ ] Update `README.md` Quickstart and command list.
- [ ] Update `AGENTS.md` with shared projection/API/validation notes.

### Phase 6 — Validation pass

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo run -p lazyadmin-cli -- search hermes --json`
- [ ] `cargo run -p lazyadmin-cli -- tui --headless --json | jq .view_model.search`
- [ ] `cargo run -p lazyadmin-cli -- web --port 0 --no-open` and smoke `curl /api/search?q=5432`.
- [ ] Run dogfood-tui against the rebuilt binary to capture fresh evidence; attach to the issue.
- [ ] Verify telemetry logs/spans for CLI/Web/TUI/runtime search paths in local dev.

## Risks / open questions

- **`/` vs single-letter filter keys on Listeners.** The Listeners view today uses `a/p/c/o/u/t` as single-key filters. We are explicitly keeping those — search only auto-focuses at launch and via `/`. Risk: a user expects to "just type" after navigating away from launch. Mitigated by an always-visible search bar with `Press / to search` placeholder when blurred.
- **`q` while typing in the input.** While `InputMode::Search`, `q` is a literal character. Existing Filter mode already established this precedent.
- **Snapshot freshness.** Search runs on the cached `app.snapshot` in TUI/Web. If the snapshot poll is mid-flight, results can be one tick stale — same as every other view.
- **Large hosts.** Cap each group at `DEFAULT_SEARCH_LIMIT` / `MAX_SEARCH_LIMIT`; expose total/returned/truncated so renderers can show `… +N more`.
- **Process pid 0 / portless aliases.** Portless adapter emits `pid = 0` aliases — process hits filter these out to avoid noise. Listener hits are unaffected.
- **System-service hiding parity.** TUI search must respect `show_system`; CLI/Web intentionally search full snapshots. Tests should cover both.
- **Folding `InputMode::Filter`.** Per-view filter UX changes because `/` now means global search. Keep keybinding action names for compatibility and dogfood specifically around Listeners chips + global query.
- **Web search vs palette.** `/` changes from palette to global search. Preserve palette access with `Ctrl/Cmd+K` and updated button copy to avoid losing fast navigation.

## Out of scope

- Workload / project / manager / doctor as independent search result groups (follow-up issue).
- Highlighted match rendering in result rows (data is captured via `matched_indices`; rendering is polish).
- Saved searches / search history.
- Regex / glob query syntax.
- Web UI URL-state for global search (`?q=…`) — explicitly punted to avoid colliding with current hash params.
- Multi-snapshot diff search.

## Done = closes issue (new) and unblocks the remaining checkboxes on #22

This subsumes the "Filter is fuzzy with no UI tell" item from #22 by replacing the footer-only `Filter:` prompt with a top-bar always-on global search. After this lands, close the corresponding checkbox in #22.
