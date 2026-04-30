# PLAN-15c: Web UI — Scrap AI-Slop Layout, Rebuild Around the Digest (Issue #16)

Parent: [PLAN-15](./PLAN-15-ux-overhaul.md) · Issue: [#16](https://github.com/shuv1337/lazyadmin/issues/16)

Throw out `crates/lazyadmin-web/static/index.html`. Rebuild around the digest from #14, with three top-level surfaces (Overview / Listeners / Workloads / Processes / Doctor / Metrics — ≤6 nav items), per-entity-kind inspector layouts, and zero metric-card slop.

## Hard constraints

- Loopback-only bind unchanged. No new mutating routes.
- The Axum server in `crates/lazyadmin-web/src/lib.rs` (559 lines) stays — only the static asset and the nav model change. Embed any new static assets via `include_str!` / `include_bytes!`. **No new runtime deps.**
- No frontend framework (no React, no Vue, no build step). Vanilla JS + small CSS.
- Snapshot polling and the `GET /api/snapshot` route are unchanged in shape; new routes are additive (`/api/digest`, `/api/doctor`, `/api/inspector/:kind/:id`).

## Prerequisites

- [ ] PLAN-15 Phase 0 complete (view-model module, classifier, theme tokens decided).
- [ ] PLAN-15a's `Digest` view-model + `/api/digest` endpoint shipped.
- [ ] PLAN-15b's `DoctorGroupsView` + `/api/doctor` endpoint shipped.
- [ ] PLAN-15d's `InspectorView` view-model shipped (or stubbed) with `/api/inspector/:kind/:id`.

## "Don't" list (must not appear in the final output)

Cribbed from `~/repos/shuvbot-skills/frontend-design/SKILL.md` and the issue body. The visual review fails if **any** of these are present:

- `display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr))` metric-card grids.
- A row of large accent-colored hero numbers above the main content.
- Glass / frosted / blurred borders.
- Gradient text.
- `<pre>{JSON.stringify(x, null, 2)}</pre>` anywhere a user can see it (raw JSON only behind a `Show raw` toggle for debugging).
- Sidebar with eleven entries, four of which are the same table with different filters.
- Filter input with no UI tell about the matching strategy.

## Tasks

### A. Server-side route additions

- [ ] `GET /api/digest` — serializes `Digest` from PLAN-15a.
- [ ] `GET /api/doctor` — serializes `DoctorGroupsView` from PLAN-15b.
- [ ] `GET /api/inspector?kind=…&id=…` — serializes the per-entity `InspectorView` from PLAN-15d. URL-encoded entity IDs.
- [ ] `GET /api/header_pip` — serializes the `HeaderPip` from PLAN-15 Phase 0.1, used by both the persistent header and #20.
- [ ] `GET /api/rail` — returns the canonical rail from `lazyadmin-runtime::RAIL_ENTRIES` (PLAN-15 #19), so the Web nav can't drift from the TUI rail.
- [ ] All routes loopback-bind-checked via the existing guard.
- [ ] Existing `GET /api/snapshot` left bit-identical.
- [ ] `cargo test -p lazyadmin-web` covers each new route shape (golden JSON snapshots).

### B. Static asset layout

- [ ] Replace `crates/lazyadmin-web/static/index.html` with three files:
  - `index.html` — minimal shell, mounts the app, no inline JS bundle.
  - `app.css` — palette as CSS custom properties (mirrors PLAN-15 Phase 0.3 token names: `--risk-public`, `--risk-lan`, `--marker-conflict`, etc.). Layout via CSS grid + flex; *no* `auto-fit minmax` card grids.
  - `app.js` — vanilla module ES module; ~300 lines target. One file is fine; no bundler.
- [ ] Embed all three via `include_str!` in `crates/lazyadmin-web/src/lib.rs`. Update tests that assert response bodies.
- [ ] `Cache-Control: no-store` on dev builds; long-cache + content hash in release (use a build-script-free trick: hash the embedded bytes at startup, expose `/static/app.js?h={hash}`).

### C. App shell

- [ ] Header (sticky):
  ```
  lazyadmin · localhost            ● healthy   updated 4s ago
  ```
  - Health pip slots from `HeaderPip` (PLAN-15 #20). Dot color from theme token.
  - Right-side: `⌘ K` chip opening the search palette (replaces the 11-entry sidebar nav and the `?view=…` URL hack).
- [ ] Top nav (≤6 entries; data from `/api/rail`):
  - Overview · Listeners · Workloads · Processes · Doctor · Metrics
  - Active item underlined in `accent`. Stateful via URL hash routing (`#/overview`, `#/listeners?filter=public`, etc.).
- [ ] Optional grouping (covered in #22): *Triage* (Overview, Doctor) / *Inventory* (Listeners, Workloads, Processes) / *Diagnostics* (Metrics) — subtle dim labels above each group.
- [ ] Footer/empty space deliberately left empty. No card row.

### D. Routes & pages

- [ ] `#/overview` — default. Renders `Digest` from `/api/digest`. Uses the same affirmative empty-state copy as the TUI (see PLAN-15a `EMPTY_EXPOSED` etc., re-exported as JSON constants from a new `/api/strings` endpoint *or* duplicated as JS constants but covered by a test that diffs them).
- [ ] `#/listeners` — flat table. Filter chips at top: All · Public · LAN · Conflicts · Orphans · Unowned · Tracked. Chip state in URL.
- [ ] `#/workloads` — grouped by parent (manager/runtime), not flat.
- [ ] `#/processes` — grouped by parent_pid; supports drill-in to process tree fragment via inspector.
- [ ] `#/doctor` — renders `DoctorGroupsView` with collapsed noise groups + severity chips. Mirrors PLAN-15b TUI semantics.
- [ ] `#/metrics` — mirrors PLAN-15 Phase 3 (#21): real units, captions, empty states, *no* hero metric numbers.
- [ ] `⌘K` palette: type-ahead over rail entries + listener IDs + project names + warning codes; Enter routes to the right page with the right filter applied.

### E. Inspector

- [ ] Right-pane inspector or modal-on-narrow. Per-entity-kind templated layout from PLAN-15d. **Never** a `<pre>{JSON}</pre>`.
- [ ] `Show raw` toggle on each inspector reveals the underlying snapshot fragment for debugging. Off by default. Toggle state per-session, not per-entity.
- [ ] Action buttons on the inspector are **disabled** (read-only Web UI per AGENTS.md). They show the command they *would* run as a tooltip / preview, identical to the TUI's pre-confirmation preview.
- [ ] Copy-to-clipboard affordance on entity IDs.

### F. Filter strategy

- [ ] Match the TUI choice from PLAN-15 #22 — substring default, fuzzy on `~` prefix. Render the strategy hint inline next to the filter input.
- [ ] Replace `JSON.stringify(x).toLowerCase().includes(filter)` with a typed predicate per page.
- [ ] Header shows `(matched / total)` count once a filter is active.

### G. Empty / error / degraded states

- [ ] `daemon not reachable — start with: lazyadmin web` — when `/api/snapshot` returns a connection error.
- [ ] `no listeners discovered yet` — when snapshot is fresh but empty.
- [ ] `snapshot stale (last update 47s ago)` — when `now - snapshot.observed_at > 5s`.
- [ ] `fetch failed: <reason>` — generic API error.
- [ ] No `loading snapshot…` left in the final UI as a permanent state — that's a 100ms-skeleton at most, then real data or a real error.

### H. Theming

- [ ] CSS custom properties keyed to the same token names as the TUI theme (PLAN-15 Phase 0.3). Theme switch in the future is a class swap, not a recompile.
- [ ] Default theme uses Night Owl tokens (already aligned). High-contrast and Solarized variants ship as `<theme>.css` chunks loaded lazily — but the v1 shipping change is just one theme, the others are stubbed for parity.

### I. Tests

- [ ] `crates/lazyadmin-web/tests/`:
  - [ ] `index_html_does_not_contain_pre_json_dump`.
  - [ ] `index_html_does_not_contain_metric_card_grid` (rg `auto-fit, minmax`).
  - [ ] `nav_has_at_most_six_entries`.
  - [ ] `health_pip_renders_drop_count_only_when_nonzero` (HTML assertion via headless render — or a JS unit test in a small Node/jest-free runner; if too heavy, assert via a JSON-driven render harness exposed for tests).
- [ ] Smoke test: `cargo run -p lazyadmin-cli -- web --port 0 --no-open` launches and `curl localhost:<port>/` returns 200 with the new HTML.
- [ ] `cargo test -p lazyadmin-runtime -p lazyadmin-web` green.

### J. Documentation

- [ ] Update `docs/spec.md` (or the linked spec) Web UI section to describe the new IA.
- [ ] Update `PLAN-14-read-only-webui.md` with a back-reference to this rebuild — leave PLAN-14 historical, do not re-edit it as if it never shipped.
- [ ] AGENTS.md: note that the Web UI nav is sourced from `lazyadmin-runtime::RAIL_ENTRIES`.

## Acceptance criteria (mirrors #16)

- [ ] Default route renders the digest.
- [ ] Metric-card row deleted.
- [ ] Sidebar replaced with a top nav of ≤6 entries.
- [ ] `Public`, `Conflicts`, `Orphans`, `Managers`, `Tracked runs`, `Projects`, `Warnings`, `Discovery health` are not top-level nav items.
- [ ] Inspector is templated per entity kind; no `<pre>{JSON}</pre>` in visible UI.
- [ ] Empty / error / degraded states implemented.
- [ ] Filter UI tells the user its matching strategy.
- [ ] Snapshot polling unchanged; loopback-only bind unchanged; no new mutating routes.
- [ ] Visual review: no AI-slop tells (see "Don't" list above).
- [ ] Smoke + tests pass: `cargo run -p lazyadmin-cli -- web --port 0 --no-open`, `cargo test -p lazyadmin-runtime -p lazyadmin-web`.

## Out of scope

- Mutating routes.
- Authentication / multi-host.
- Build pipeline / bundler / framework.

## Risks

| Risk | Mitigation |
| ---- | ---------- |
| Drift between TUI and Web wording. | Empty-state copy and rail entries served from `lazyadmin-runtime` via JSON endpoints; tested for parity. |
| Vanilla JS file grows past readability. | Hard ceiling 400 lines; if exceeded, split into per-page modules with native ES `import` (still no bundler). |
| CSS palette drifts from theme tokens. | Single source of truth: a `themes.json` generated from `Theme::builtins()` at build time and embedded; CSS reads via `var(--token)`. |
| Broken inspector when an entity is removed mid-poll. | `/api/inspector/:kind/:id` returns a structured `not_found` body; UI shows `Entity gone — refresh`. |

## Dogfood-evidence reproduction

After landing, capture a fresh Web UI screenshot at default load and at `#/doctor`; archive under `dogfood-web-output/`. Compare against the issue's described AI-slop sniff test for issue close-out.
