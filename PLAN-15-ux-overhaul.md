# PLAN-15: UX Overhaul — Ship Answers, Not 11 Filtered Tables

Tracking plan for issue [#13](https://github.com/shuv1337/lazyadmin/issues/13). Sequences the nine child issues (#14–#22), captures shared foundation work, and inlines full task lists for the lighter issues. Heavy issues spin out into dedicated plans:

- [PLAN-15a — Digest landing screen (#14)](./PLAN-15a-digest-landing.md)
- [PLAN-15b — Doctor warning groups (#15)](./PLAN-15b-doctor-grouping.md)
- [PLAN-15c — Web UI rebuild (#16)](./PLAN-15c-web-ui-rebuild.md)
- [PLAN-15d — Inspector pane (#17)](./PLAN-15d-inspector-redesign.md)

The plans below are checkbox-tracked so we mark progress in-place per the `lazy-implementer` workflow.

## Goals (from #13)

- Stop shipping 11 views of the same list. Ship 2–3 opinionated views that each answer one operator question, then put the long tail behind search.
- Encode meaning (exposure, ownership, project, conflict) in visual hierarchy — not in a column buried 6 columns deep.
- Make the inspector richer than the row, not a transposed copy of it.
- Make the Web UI either a true product surface or retire it for `lazyadmin export`.
- Stop looking like a generic AI-generated admin panel (Web UI) and stop looking like a generic Ratatui template (TUI).

## Non-goals

- No new adapters, no new data sources.
- No mutating actions beyond what we already ship (`kill`, `free-port`, `pause`).
- No theming work beyond what #18 needs.
- No JSON contract changes — `lazyadmin export --json` / `lazyadmin doctor --json` stay byte-stable.

## What we keep

1. Action safety (typed-verb confirmation, dry-run preamble, Esc-cancels).
2. Provenance is shown, not hidden.
3. Refuse-mode under 60 columns.

## Architectural ground truth (post-recon)

- **`crates/lazyadmin-runtime/src/lib.rs`** is currently 168 lines — only snapshot building + the live-refresh task. The shared view-model layer this plan needs **does not yet exist** there. We will grow it.
- **`crates/lazyadmin-tui/src/lib.rs`** is 4,957 lines and owns `ViewKind`, view-model assembly, and rendering all in one file. It currently knows how to project from `Snapshot` directly. The plan keeps the TUI rendering local but moves the *projection* into `lazyadmin-runtime`.
- **`crates/lazyadmin-web/`** is one 17-line `static/index.html` plus a 559-line Axum server. The HTML/JS is fully throwaway; the Axum routes stay.
- JSON contract surface: `Snapshot`, `Warning`, `WarningSeverity`, `EntityRef`, `Provenance` in `crates/lazyadmin-core/src/model/mod.rs`. Every grouping/digest output is a *view-model transform* — never a model change.

## Sequencing

Foundation → Structural → Polish → Cleanup, mirroring #13's grouping. Each tier ships independently; later tiers consume earlier outputs.

```
Phase 0  Shared foundation         (this plan, §"Phase 0")
Phase 1  Foundation issues          #14, #15           (PLAN-15a, PLAN-15b)
Phase 2  Structural rebuild         #16, #17, #19      (PLAN-15c, PLAN-15d, here)
Phase 3  Visual + chrome polish     #18, #20, #21      (here)
Phase 4  Cleanup                    #22                (here)
Phase 5  Tracking close             AGENTS.md, dogfood, definition-of-done
```

Phase 0 is mandatory before any user-visible work begins. Everything in phases 1–4 can move in parallel within a phase but never across.

---

## Phase 0 — Shared foundation (mandatory before #14/#15)

Goal: stand up the shared infrastructure every later issue depends on. No user-visible changes yet. Lands as a single PR (or one PR per bullet group, but in dependency order).

### 0.1 — Grow `lazyadmin-runtime` into the view-model layer

- [x] Create `crates/lazyadmin-runtime/src/view_model/` module tree:
  - [x] `mod.rs` — re-exports + the `RuntimeViewModels { digest, inspector, doctor_groups, header_pip }` aggregate.
  - [x] `digest.rs` — `Digest` struct (fields per #14: `exposed`, `conflicts`, `your_projects`, `triage`); pure projection from `&Snapshot`.
  - [x] `inspector.rs` — per-entity-kind `InspectorView` enum (`Listener`, `Workload`, `Process`, `Project`, `Manager`, `TrackedRun`, `WarningGroup`); see PLAN-15d for fields.
  - [x] `doctor_groups.rs` — `WarningGroup { code, severity, count, sample_entities, suggested_action, tier, expanded }` plus the grouping function; see PLAN-15b.
  - [x] `header_pip.rs` — `HeaderPip { adapters: AdapterHealth, freshness: SnapshotFreshness, drops: Option<DropRate> }` consumed by #20 + #16.
  - [x] `tests.rs` — initial empty/warning fixture tests projecting through every view-model (busy/degraded goldens still pending).
- [ ] Add `lazyadmin-runtime` golden fixtures under `crates/lazyadmin-runtime/testdata/` reusing `testdata/snapshots/empty.json` plus a new `busy.json` derived from a captured live snapshot (redacted of host-specific bits — reuse the redaction helpers in `lazyadmin-core::redact`).
- [ ] Wire `cargo test -p lazyadmin-runtime` into the workspace AGENTS.md "validation commands" block.

### 0.2 — Warning code → tier/remediation table in core

- [x] In `crates/lazyadmin-core/src/doctor.rs` (or a new `doctor/registry.rs`), add a `WarningCodeMeta { code: &'static str, tier: WarningTier, remediation: &'static str, label: &'static str }` registry, one row per shipped warning code.
  - [x] Audit existing emitters first: `rg "WarningCode\|Warning \{ code:" crates/lazyadmin-*` and enumerate every code we currently produce.
  - [x] `WarningTier` enum: `Critical | Actionable | Noise`.
- [x] Public function: `pub fn classify(code: &str) -> WarningCodeMeta` — falls back to `tier: Actionable, remediation: "inspect details"` for unknown codes (so unrecognized codes don't get silently demoted to noise).
- [x] Unit tests for every shipped code → expected tier/label.
- [x] **No JSON contract change**: this is a metadata lookup over an existing field, not a new field on `Warning`.

### 0.3 — Theme palette extension

- [x] In `crates/lazyadmin-tui/src/lib.rs` extend `Theme` with new semantic slots:
  - [x] `risk_public`, `risk_lan`, `risk_loopback`
  - [x] `marker_conflict`, `marker_tracked`, `marker_project`
  - [x] `system_noise`
  - [x] `pip_ok`, `pip_warn`, `pip_error` (used by #20 header pip)
- [x] Update `palette_entries` and the `theme_builtins_validate_and_downgrade` test (per AGENTS.md note).
- [x] Map sensible defaults in all three builtins (`default-dark`/`night-owl`, `default-light`/`night-owl-light`, `high-contrast`, `solarized-dark`). Keep existing rendering visually similar where possible — only #18 actually flips usage.
- [x] Mirror the slots in the Web UI as CSS custom properties (file lands in #16 but the palette token names are decided here and documented in `docs/themes.md`).

### 0.4 — Header pip + status-channel split scaffolding

- [x] In `lazyadmin-tui` introduce a `StatusChannel` enum: `HeaderPip`, `Toast { ttl: Duration }`, `ModalHint`. Existing `set_status`-style call sites stay no-op redirected to `Toast` for now (so we don't regress before #20).
- [x] Add a `Toast` queue on `App` with last-render-tick timestamps; rendering is wired in #20.
- [x] No visual change yet. Tests only confirm channel routing compiles + dispatches.

### 0.5 — Rollup acceptance for Phase 0

- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [x] `cargo test --workspace` green; new `lazyadmin-runtime::view_model` tests green.
- [x] `cargo run -p lazyadmin-cli -- export --json` produces valid JSON; byte-stability preserved by no schema/model changes (golden diff still pending).
- [x] `cargo run -p lazyadmin-cli -- doctor --json` produces valid JSON; byte-stability preserved by no report schema changes (golden diff still pending).
- [x] AGENTS.md updated to mention the new `lazyadmin-runtime::view_model` module and the warning-classifier registry.

---

## Phase 1 — Foundation issues

Both issues are written up in dedicated plans. They can land in either order but #14 will lean on #15's `WarningGroup` count for its triage section, so prefer #15 first or stub the count.

- [x] **#14 — Digest landing screen.** See [PLAN-15a-digest-landing.md](./PLAN-15a-digest-landing.md).
- [x] **#15 — Doctor: aggregate warnings by `(code, severity)` and rank by actionability.** See [PLAN-15b-doctor-grouping.md](./PLAN-15b-doctor-grouping.md).

### Phase 1 acceptance

- [x] Cold `lazyadmin tui` shows the digest as the default view.
- [x] `lazyadmin doctor` (TUI view) shows grouped warnings with affirmative empty state.
- [x] `lazyadmin export --json` and `lazyadmin doctor --json` outputs are unchanged byte-for-byte from pre-#14.
- [ ] Fresh dogfood TUI run captured under `dogfood-tui-output/` shows the digest landing.

---

## Phase 2 — Structural rebuild

#16 and #17 are written up in dedicated plans. #19 is straightforward enough to track inline.

- [x] **#16 — Web UI rebuild.** See [PLAN-15c-web-ui-rebuild.md](./PLAN-15c-web-ui-rebuild.md). Static UI rebuilt as `index.html` + `app.css` + `app.js`; new `/api/header_pip` and `/api/inspector` routes; rail mirrors `RAIL_ENTRIES`; templated inspector, no `<pre>{JSON}</pre>` in the visible UI.
- [x] **#17 — Inspector pane.** See [PLAN-15d-inspector-redesign.md](./PLAN-15d-inspector-redesign.md). Per-kind `InspectorView` view-models, `/api/inspector` typed shape, Web rendering, confidence-signal classification, and the TUI `render_inspector` rewrite are shipped.

### #19 — Collapse the rail to ~6 verbs (inline)

- [x] Audit current rail in `crates/lazyadmin-tui/src/lib.rs`: `ViewKind` enum (line ~164) and the rail-group strings (search `"All/Everything"`, `"Public listeners"`, etc., around lines 1929–1990).
- [x] Add new `ViewKind::Overview` (digest from #14), `ViewKind::Workloads`, `ViewKind::Listeners` (the consolidated table-with-chips view).
- [x] Mark `ViewKind::Ports`, `Public`, `Conflicts`, `Orphans`, `TrackedRuns` for migration:
  - Keep them addressable via `ViewKind` so `lazyadmin tui --view public` and existing keybindings still work.
  - **Do not** show them in the rail. Replace their rail entries with **filter chips** inside `Listeners`.
- [x] Add a `ListenerFilter` enum (`All`, `Public`, `Conflicts`, `Orphans`, `Unowned`, `Tracked`). The chip toolbar is rendered inside `render_view_kind` for `ViewKind::Listeners`. Each chip toggles a filter predicate over `snapshot.listeners`.
- [x] Remove the four disabled adapter rail entries (`Docker/Compose`, `Podman`, `systemd:user`, `systemd:system`, `Direct processes`). Their data moves into:
  - The header status pip (#20): `adapters: 2/6 active`.
  - A new section in the `Metrics` view: `Discovery health` table.
- [x] Update `cli_hints_for_view` and `narrow_refusal_message` for the new view list.
- [x] First-run hint banner: when the user first lands on `ViewKind::Listeners`, show a one-line dim hint: `Filters now live as chips — try [P]ublic, [C]onflicts, [/] to search.` Auto-dismisses after first chip toggle or on `?` help.
- [x] Update the help overlay (`?`) and `lazyadmin tui --help` to document the new mapping.
- [x] Tests:
  - [x] Rail enumeration test: `rail_has_at_most_eight_entries`.
  - [x] No `[hidden]`-clipped string anywhere in rail rendering at 70/90/120/160 cols.
  - [x] `ViewKind::Public` programmatic entry still works (`build_view_model_with_state` path covered).
  - [x] Chip toggle predicate tests against `testdata/snapshots/busy.json`.
- [x] Web UI nav (#16) consumes the same rail order — single source of truth lives in `lazyadmin_runtime::view_model::RAIL_ENTRIES` (`RailEntry { id, label }`) exposed from `lazyadmin-runtime`.

### Phase 2 acceptance

- [x] TUI rail has ≤8 entries; no disabled adapter rows.
- [x] Chips inside `Listeners` reproduce the previous filtered-view results 1:1 (golden test).
- [x] Web UI default route is the digest; no metric-card row; inspector is templated, not `<pre>{JSON}</pre>`.
- [x] Inspector layouts implemented for all 7 entity kinds; no `-` rows. Runtime view-models, Web rendering, and TUI section rendering all consume the same inspector contract.

---

## Phase 3 — Visual + chrome polish (parallelizable)

These three issues are mostly mechanical now that Phase 0 landed the palette + status channels.

### #18 — Visual hierarchy: encode signals in row weight

- [ ] In the row renderer for `Listeners` (and Process Tree, Workloads, the digest sections), add a leading-glyph + color prefix slot.
  - Public exposure: `●` colored `risk_public`.
  - LAN exposure: `●` colored `risk_lan`.
  - Loopback: no prefix, default weight.
- [ ] Add a left-border marker slot independent of the prefix:
  - Conflict: `┃` in `marker_conflict`.
  - Tracked-run / project member: `▎` in `marker_tracked` / `marker_project`.
- [ ] Header counts colored to match: `120 public` in `risk_public`, `12 LAN` in `risk_lan`.
- [ ] System-noise dimming: rows owned by known-system daemons (DNS resolver, journald, etc.) render in `system_noise`. Toggle remains `S` (existing `Command::ToggleSystem`).
- [ ] Monochrome safety: every signal uses a distinct glyph + weight, not just color. Add a TUI test that runs rendering with `PaletteMode::Monochrome` and asserts every row class is still distinguishable by glyph.
- [ ] Colorblind safety: pick a red/orange pair with distinct luminance, *or* ship a `colorblind-safe` theme variant alongside `default-dark`. Document in `docs/themes.md`.
- [ ] Web UI mirrors the same encoding via CSS classes (`.risk-public`, `.marker-conflict`, etc.) bound to the CSS custom properties from Phase 0.3.
- [ ] Theme tests: `theme_builtins_validate_and_downgrade` covers the new slots; new test `risk_glyphs_present_without_color` for monochrome.
- [ ] Update `dogfood-tui-output` fixture screenshots after landing.

### #20 — Footer split + persistent header health pip

- [ ] **Footer width-padding fix** lifted from `draw_three_pane` into the top-level `render_app` footer (lib.rs ~3540). Single shared helper `pad_to_width(line, width)` so SinglePane / InspectorTab / ThreePane / refuse-mode all use it.
- [ ] Footer becomes static, context-sensitive key hints only:
  - `[?] help   [:] palette   [/] filter   [enter] inspect   [q] quit`
  - Per-focus variants for `Pane::Groups`, `Pane::Rows`, `Pane::Inspector`, plus modal/filter-input modes.
- [ ] Toast overlay rendered just above the footer (`area.bottom() - 2 .. area.bottom() - 1`) with `Block::clear`. TTL default 2s; typing during a toast cancels dismissal.
- [ ] **Remove** `— refresh may lag` from the footer entirely. Stale snapshot (>5s) → header pip orange dot + `12s ago`. Fresh → green dot + `4s ago` or no freshness slot.
- [ ] Header pip slots (driven by `HeaderPip` from Phase 0.1):
  - `● healthy` / `⚠ events dropped 1 (last 60s)` / `⚠ refresh stale (12s)`
  - `adapters: 2/6 active`
  - `last update 4s ago`
- [ ] Confirmation modals (`kill`, `free-port`) move their hint string inside the modal block, not the footer.
- [ ] Migrate every existing `set_status` / `App::status` call site to the right channel (Phase 0.4 stub becomes real here).
- [ ] Tests:
  - [ ] `footer_padded_to_full_width_in_every_layout` (SinglePane/InspectorTab/ThreePane/refuse).
  - [ ] `toast_dismisses_after_ttl` and `toast_dismissal_paused_during_input`.
  - [ ] `no_residue_when_long_message_replaced_by_short_message` — direct repro of the bug #8 family.
  - [ ] `header_pip_renders_drop_count_only_when_nonzero`.
- [ ] Web UI (#16) gains the same header pip — slots wired to the existing `HeaderPip` view-model.

### #21 — Metrics view: real units, real labels, honest empty states

- [ ] **Events dropped** rendered as a rate over a rolling window: `27 / 4,200 events dropped in last 60s = 0.6%`, plus a sparkline.
  - If the window is unobservable (stateless run), render `drop counter unavailable in stateless run`.
  - Implementation: extend `EventDropCounter` with a ring of `(timestamp, count)` samples (configurable depth, default 60). The current single-counter API stays for backwards-compat callers.
- [ ] **Adapter event rate** affirmative empty state: `No events in last 60s — adapter is idle (this is normal).` Drop the empty box.
- [ ] **Listeners histogram** axis labels: full words `Listeners` / `Public` / `Conflicts` / `Orphans`. If they don't fit horizontally, rotate the chart 90° (horizontal bars) — Ratatui supports this via `BarChart::direction`.
- [ ] Per-chart caption (one dim line) explaining the metric and the action to take if it's bad. Captions live in `crates/lazyadmin-core/src/doctor/metrics_glossary.rs` (or a small `metrics_glossary.toml` loaded once via `include_str!`).
- [ ] Web UI Metrics page mirrors layout, captions, units, empty states.
- [ ] Tests in `cargo test -p lazyadmin-tui metrics`:
  - [ ] `events_dropped_rate_with_nontrivial_denominator_renders_correctly`.
  - [ ] `empty_adapter_event_rate_shows_idle_message`.
  - [ ] `listener_histogram_axis_uses_full_words`.

### Phase 3 acceptance

- [ ] All listener/process tables across both UIs encode exposure/conflict/ownership in row weight, not in a 6th column.
- [ ] Footer never leaks residue; header pip is the persistent health surface.
- [ ] Metrics view reads as a confidence-builder, not a build break.

---

## Phase 4 — Cleanup

### #22 — Polish pass

Each checkbox is a small mechanical PR or a polish-bundle PR.

- [ ] **Filter affordance.** Pick one strategy (substring default, fuzzy on `~` prefix). Update prompt label, append `(matched / total)` count to header. Apply to both TUI and Web UI filter inputs.
- [ ] **`l` keybinding overload.** Rebind: `l` = global Logs, `Shift+L` = logs-for-selection. Update inspector action listing accordingly. Document in help overlay + `docs/keybindings.md`.
- [ ] **Action labels** (covered structurally by #17). Polish lands here for cosmetic consistency in narrow inspector panes.
- [ ] **`:theme` (no arg).** Replace `unknown command: theme` with `theme: missing argument. usage: theme <name>` and list available themes from `Theme::builtins()`.
- [ ] **`Logs unavailable for direct processes`.** Shorten to `Logs — none (direct process)`.
- [ ] **`lan/public` scope value.** Pick a single representation. Recommendation: split into two boolean fields `reachable_lan: bool, reachable_public: bool` on the *view-model* (not snapshot), and render as separate chips. **Snapshot JSON unchanged** — this is presentation only.
- [ ] **TUI header double-spacing.** Promote `120 public` to a colored chip (uses #18 palette); demote the rest to a thinner secondary line.
- [ ] **Process Tree count.** Add `1,247 processes, 18 roots` to the pane title.
- [ ] **Web UI nav grouping.** Group surviving entries (post-#19) into *Triage / Inventory / Diagnostics* with subtle headings.
- [ ] **Web UI status copy.** Implement: `daemon not reachable — start with: lazyadmin web`, `no listeners discovered yet`, `snapshot stale (last update 47s ago)`, `fetch failed: <reason>`.
- [ ] Help overlay (`?`) reflects every rebound key.
- [ ] No regression in `cargo test --workspace`.

### Phase 4 acceptance

- [ ] All #22 checkboxes closed.
- [ ] Help overlay and `docs/keybindings.md` consistent with shipped binds.

---

## Phase 5 — Tracking close

- [ ] Fresh dogfood TUI run on the same host as the v0.4 baseline:
  - [x] Default screen is the digest (#14).
  - [ ] Rail shows ≤8 entries (#19).
  - [x] Doctor view shows grouped warnings (#15).
  - [ ] Inspector is full-fidelity (#17).
  - [ ] Footer carries no residue under repeated state churn (#20).
- [ ] Fresh Web UI screenshot does **not** trip the *AI-generated admin panel* sniff test:
  - No metric-card row.
  - No `<pre>{JSON}</pre>` inspector.
  - No 11-entry sidebar of duplicated filters.
- [ ] `cargo test --workspace` green.
- [ ] Validation block in AGENTS.md — every command listed there runs clean:
  ```
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
  cargo run -p lazyadmin-cli -- doctor --json
  cargo run -p lazyadmin-cli -- events --once --json
  cargo run -p lazyadmin-cli -- tui --headless --json
  cargo test -p lazyadmin-runtime -p lazyadmin-web
  cargo run -p lazyadmin-cli -- web --port 0 --no-open
  cargo test -p lazyadmin-tui render_views
  cargo test -p lazyadmin-tui live_refresh
  cargo test -p lazyadmin-tui process_tree
  cargo test -p lazyadmin-tui metrics
  cargo test -p lazyadmin-tui theme
  cargo test -p lazyadmin-tui keybindings
  ```
- [ ] AGENTS.md updated:
  - New `crates/lazyadmin-runtime/src/view_model/` module described.
  - Rail collapsed to ≤8 entries; how to find old views.
  - Warning classifier registry described.
  - Theme palette new slots listed alongside Night Owl notes.
  - New status-channel architecture (`HeaderPip` / `Toast` / `ModalHint`) documented.
- [ ] Close issues #14–#22 once each issue's plan is fully checked off.
- [ ] Close tracking issue #13 once Phase 5 is complete.

---

## Risk register

| Risk | Mitigation |
| ---- | ---------- |
| 4,957-line `crates/lazyadmin-tui/src/lib.rs` is a merge-conflict hot spot. | Phases 1–4 each touch disjoint regions: #14/#19 touch `ViewKind`, #17 touches inspector renderers, #18 touches row renderers, #20 touches footer/header. Land Phase 0 first, then sequence one phase per merge to avoid conflicts. |
| `lazyadmin-runtime::view_model` becomes a god module. | Sub-modules per concern (`digest.rs`, `inspector.rs`, `doctor_groups.rs`, `header_pip.rs`); each ≤300 lines target. |
| Warning classifier drifts from emitter changes. | New emitter PRs must add a row to the registry; CI lint: `rg "Warning \{ code:" crates/lazyadmin-* | check_against_registry`. |
| Web UI rebuild looks like a different AI-slop template. | PLAN-15c includes an explicit "do-not" list cribbed from `~/repos/shuvbot-skills/frontend-design/SKILL.md` plus a sniff-test checklist. |
| Stateless `lazyadmin doctor` runs can't observe drop rates. | Existing AGENTS.md note already covers this; #21's empty-state copy makes it explicit. |
| Snapshot JSON contract drift. | Every plan re-asserts: `lazyadmin export --json` and `lazyadmin doctor --json` are byte-stable. Phase-0 + Phase-5 acceptance both diff against pre-overhaul golden output. |

## Open questions

- Should `ViewKind::Overview` be the new default or should we keep `Everything` and add a `--view overview` opt-in for one release cycle? Default is more honest to #13's intent; opt-in is safer for muscle memory. **Recommendation: ship as default.**
- Do we want the digest's "Your projects" section to include projects with zero running listeners (informational) or only running ones (action-focused)? **Recommendation: only running ones in the default; `Projects` rail entry shows all.**
- For #18's monochrome safety, should `default-dark` keep its current colors and we add `colorblind-safe` as a separate theme, or do we adjust `default-dark` for luminance contrast and label the original as `default-dark-legacy`? **Recommendation: separate theme; don't perturb the canonical Night Owl baseline.**
