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
- [x] PLAN-15a's `Digest` view-model + `/api/digest` endpoint shipped.
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

- [x] `GET /api/digest` — serializes `Digest` from PLAN-15a.
- [x] `GET /api/doctor` — serializes `DoctorGroupsView` from PLAN-15b.
- [x] `GET /api/inspector?kind=…&id=…` — serializes the per-entity `InspectorView` from PLAN-15d. URL-encoded entity IDs.
- [x] `GET /api/header_pip` — serializes the `HeaderPip` from PLAN-15 Phase 0.1, used by both the persistent header and #20.
- [x] `GET /api/rail` — returns the canonical rail from `lazyadmin-runtime::RAIL_ENTRIES` (PLAN-15 #19), so the Web nav can't drift from the TUI rail.
- [x] All routes loopback-bind-checked via the existing guard.
- [x] Existing `GET /api/snapshot` left bit-identical.
- [x] `cargo test -p lazyadmin-web` covers each new route shape (golden JSON snapshots).

### B. Static asset layout

- [x] Replace `crates/lazyadmin-web/static/index.html` with three files:
  - `index.html` — minimal shell, mounts the app, no inline JS bundle.
  - `app.css` — palette as CSS custom properties (mirrors PLAN-15 Phase 0.3 token names: `--risk-public`, `--risk-lan`, `--marker-conflict`, etc.). Layout via CSS grid + flex; *no* `auto-fit minmax` card grids.
  - `app.js` — vanilla module ES module; one file, no bundler.
- [x] Embed all three via `include_str!` in `crates/lazyadmin-web/src/lib.rs`. Update tests that assert response bodies.
- [x] `Cache-Control: no-store` on dev builds (the only mode v1 ships); long-cache + content hash deferred until we actually have a release pipeline.

### C. App shell

- [x] Header (sticky):
  ```
  lazyadmin · localhost            ● healthy   updated 4s ago
  ```
  - Health pip slots from `HeaderPip` (PLAN-15 #20). Dot color from theme token.
  - Right-side: `/` chip opening the search palette (replaces the 11-entry sidebar nav and the `?view=…` URL hack).
- [x] Top nav (≤6 entries; data from `/api/rail`):
  - Overview · Listeners · Workloads · Processes · Doctor · Metrics
  - Active item underlined in `accent`. Stateful via URL hash routing (`#/overview`, `#/listeners?filter=public`, etc.).
- [x] Optional grouping (covered in #22): *Triage* (Overview, Doctor) / *Inventory* (Listeners, Workloads, Processes) / *Diagnostics* (Metrics) — subtle dim labels above each group.
- [x] Footer/empty space deliberately left empty. No card row.

### D. Routes & pages

- [x] `#/overview` — default. Renders `Digest` from `/api/digest`. Empty-state copy comes straight off the `Digest` view-model (`empty_copy` fields), so the TUI and Web wording can't drift.
- [x] `#/listeners` — flat table. Filter chips at top: All · Public · LAN · Conflicts · Orphans · Unowned · Tracked. Chip state in URL.
- [x] `#/workloads` — grouped by parent (manager/runtime), not flat.
- [x] `#/processes` — grouped by parent_pid; supports drill-in to process inspector. (Process *tree* fragment view itself is a TUI feature; the Web inspector exposes the full process record.)
- [x] `#/doctor` — renders `DoctorGroupsView` with collapsed noise groups + severity chips. Mirrors PLAN-15b TUI semantics.
- [x] `#/metrics` — mirrors PLAN-15 Phase 3 (#21): real units, captions, empty states, *no* hero metric numbers.
- [x] `/`-key palette: type-ahead over rail entries + listener IDs + project names + warning codes; Enter routes to the right page with the right filter applied. (Cmd/Ctrl+K is also bound.)

### E. Inspector

- [x] Right-pane inspector (bottom sheet on narrow). Per-entity-kind templated layout from PLAN-15d. **Never** a `<pre>{JSON}</pre>`.
- [x] `show raw` toggle on each inspector reveals the underlying snapshot fragment for debugging. Off by default. Toggle state per-session, not per-entity.
- [x] Action buttons on the inspector are **disabled** (read-only Web UI per AGENTS.md). They show the command they *would* run as a tooltip / preview.
- [x] Copy-to-clipboard affordance on entity IDs.

### F. Filter strategy

- [x] Match the TUI choice from PLAN-15 #22 — substring default, fuzzy on `~` prefix. Strategy hint rendered inline next to the filter input.
- [x] Replaced `JSON.stringify(x).toLowerCase().includes(filter)` with a typed haystack per page.
- [x] Header shows `(matched / total)` count via the page-head subtle text.

### G. Empty / error / degraded states

- [x] `daemon not reachable — start with: lazyadmin web` — when fetching fails entirely.
- [x] `no listeners discovered yet` — surfaced via the per-page table empty row.
- [x] `snapshot stale (last update Ns ago)` — header pip flips warn + page banner when `freshness.age_seconds > 5`.
- [x] `fetch failed: <reason>` — generic API error banner.
- [x] `loading snapshot…` only renders before the first poll completes; replaced with real data or a real error.

### H. Theming

- [x] CSS custom properties keyed to the same token names as the TUI theme (PLAN-15 Phase 0.3). Theme switch is now a `body` class swap, not a recompile.
- [x] Default theme uses Night Owl tokens. High-contrast / Solarized variants are not bundled in v1; the token surface is in place so they can be added by appending a `body.theme-…` block.

### I. Tests

- [x] In `crates/lazyadmin-web/src/lib.rs::tests`:
  - [x] `index_html_does_not_contain_pre_json_dump`.
  - [x] `app_css_does_not_contain_metric_card_grid` (substring asserts on `auto-fit`, `backdrop-filter`, gradient text).
  - [x] `rail_constant_has_at_most_six_entries`.
  - [x] `header_pip_route_returns_expected_shape`.
  - [x] `inspector_route_returns_404_for_unknown_id`.
  - [x] `static_assets_are_served_with_no_store_in_dev`.
  - [ ] `health_pip_renders_drop_count_only_when_nonzero` — deferred. The pip is rendered in JS; the corresponding Rust-side guarantee (`drops` only present when nonzero) is enforced by `HeaderPip::from_snapshot`'s `.filter(|d| *d > 0)` and is exercised indirectly by the existing pip-shape test.
- [x] Smoke test: `cargo run -p lazyadmin-cli -- web --port 0 --no-open` launches and `curl localhost:<port>/` returns 200 with the new HTML.
- [x] `cargo test -p lazyadmin-runtime -p lazyadmin-web` green.

### J. Documentation

- [ ] Update `docs/spec.md` (or the linked spec) Web UI section to describe the new IA. *(deferred — separate doc PR; the live shape is captured in this plan.)*
- [ ] Update `PLAN-14-read-only-webui.md` with a back-reference to this rebuild — leave PLAN-14 historical, do not re-edit it as if it never shipped. *(deferred to the same doc PR.)*
- [x] AGENTS.md: note that the Web UI nav is sourced from `lazyadmin-runtime::RAIL_ENTRIES` (already noted in the existing AGENTS.md PLAN-14 / PLAN-15 #19 entry; refreshed with the new routes).

## Acceptance criteria (mirrors #16)

- [x] Default route renders the digest.
- [x] Metric-card row deleted.
- [x] Sidebar replaced with a top nav of ≤6 entries.
- [x] `Public`, `Conflicts`, `Orphans`, `Managers`, `Tracked runs`, `Projects`, `Warnings`, `Discovery health` are not top-level nav items.
- [x] Inspector is templated per entity kind; no `<pre>{JSON}</pre>` in visible UI (raw view is debug-only behind a session toggle).
- [x] Empty / error / degraded states implemented.
- [x] Filter UI tells the user its matching strategy.
- [x] Snapshot polling unchanged; loopback-only bind unchanged; no new mutating routes.
- [x] Visual review: no AI-slop tells (see "Don't" list above) — enforced in tests.
- [x] Smoke + tests pass: `cargo run -p lazyadmin-cli -- web --port 0 --no-open`, `cargo test -p lazyadmin-runtime -p lazyadmin-web`.

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
