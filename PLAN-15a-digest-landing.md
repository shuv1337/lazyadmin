# PLAN-15a: Digest Landing Screen (Issue #14)

Parent: [PLAN-15](./PLAN-15-ux-overhaul.md) · Issue: [#14](https://github.com/shuv1337/lazyadmin/issues/14)

Replace the default `Everything / Listeners` 1,749-row landing view with an opinionated **triage digest** in both the TUI and the Web UI. Each digest section answers one operator question and caps at ~10 rows with a `[view all N →]` drill-in.

## Prerequisites

- [x] Phase 0 of PLAN-15 complete (`lazyadmin-runtime::view_model` module exists; warning classifier landed; theme slots reserved).
- [x] PLAN-15b's `WarningGroup` view-model exists or has a stub returning `(actionable_count, noise_groups_count)` — the triage section consumes those numbers.

## Data sources (no model changes)

Everything below projects from the existing `Snapshot`:

- `snapshot.listeners` + `Listener.exposure` for the Exposed section.
- `snapshot.warnings` filtered by `code == "port_conflict"` (and similar) plus the per-listener owner count ≥ 2 for Conflicts.
- `snapshot.projects` joined to `snapshot.workloads` and `snapshot.listeners` for "Your projects".
- `snapshot.warnings` grouped via PLAN-15b for the Triage queue counts.

## Tasks

### A. View-model

- [x] In `crates/lazyadmin-runtime/src/view_model/digest.rs`:
  ```rust
  pub struct Digest {
      pub exposed: ExposedSection,
      pub conflicts: ConflictsSection,
      pub your_projects: ProjectsSection,
      pub triage: TriageSection,
  }
  pub struct ExposedRow {
      pub listener_id: String,
      pub port: Option<u16>,
      pub bind: String,        // "0.0.0.0:5432" preformatted for display
      pub exposure: Exposure,  // Public | Lan
      pub owner_label: String, // "node pid 17385" or "—" + unowned flag
      pub owner_pid: Option<i32>,
      pub project: Option<String>,
      pub extra_ports: usize,  // when deduped by owner_pid, count of folded ports
      pub risk_rank: u8,       // for stable sort + tests
  }
  pub struct ExposedSection {
      pub rows: Vec<ExposedRow>,
      pub total_public: usize,
      pub total_lan: usize,
      pub unowned_count: usize,
      pub view_all_target: ViewKind, // ViewKind::Listeners with chip Public
  }
  // … plus ConflictsSection, ProjectsSection, TriageSection
  ```
- [x] Pure function `pub fn build_digest(snapshot: &Snapshot, classifier: &WarningClassifier) -> Digest`.
- [x] Ranking heuristic for `exposed.rows` (per #14):
  - [x] Sort key: `(unowned desc, public desc, project_known desc, port asc)`.
  - [x] Cap at 10 rows.
  - [x] Dedupe by `owner_pid`: when one process holds N ports, fold to a single row with `extra_ports = N - 1`.
  - [x] Stable order on ties so tests don't flake.
- [x] Conflicts section: top 5 by `(severity desc, owner_count desc, port asc)` from existing `Conflicts` view logic — extract that predicate from `lazyadmin-tui` and lift it into `lazyadmin-core::doctor` if it isn't there already.
- [x] Projects section: include only projects with ≥1 owning workload that has ≥1 active listener; sort by `(listener_count desc, last_seen desc)`. Cap at 10.
- [x] Triage section: consumes `WarningGroupSummary { actionable: usize, noise_groups: usize }` from PLAN-15b.
- [x] Affirmative empty states are part of the data shape:
  - `ExposedSection::rows.is_empty()` ⇒ caller renders `Nothing exposed beyond loopback. ✓`.
  - Same for Conflicts (`Nothing contended.`), Projects (`No active projects detected.`), Triage (`Everything's clean — last check {age}.`).
  - Encode the exact copy strings as `pub const EMPTY_EXPOSED: &str = "Nothing exposed beyond loopback. ✓"` so TUI and Web share wording.
- [x] Tests in `view_model/digest.rs`:
  - [x] `digest_empty_snapshot_has_all_affirmative_empty_states`.
  - [x] `exposed_dedupes_owner_pid_and_counts_extra_ports`.
  - [x] `exposed_ranking_unowned_before_owned_before_known_system`.
  - [x] `digest_caps_each_section_at_ten_rows`.
  - [x] Golden against `testdata/snapshots/busy.json`.

### B. TUI integration

- [x] Add `ViewKind::Overview` (PLAN-15 §Phase 2 / #19 also reserves this). Make it the new `#[default]` for `ViewKind`.
- [x] Move the previous `Everything` to be addressable but no longer the default. Keep the keybinding (palette `view all`) and the `--view everything` CLI flag.
- [x] Render path:
  - [x] In `render_view_kind` add a `ViewKind::Overview` branch that calls a new `render_digest(area, &vm.digest, theme)`.
  - [x] Layout: vertical stack of the four sections; section header in `accent`; cap counts in the section header (`EXPOSED 12 (10 shown)`).
  - [x] Per-row rendering reuses the prefix-glyph helpers from #18 once available; use plain glyphs as a stopgap while #18 is in flight.
  - [x] `[view all N →]` is a tab-stop; pressing Enter sets `app.active_view = section.view_all_target` and applies any pre-set chip filter (#19).
- [x] Refuse-mode (<60 cols): digest collapses to one-line section summaries (`EXPOSED 12 · CONFLICTS 1 · PROJECTS 2 · TRIAGE 4 actionable`).
- [x] Tests:
  - [x] `digest_renders_at_120_90_70_cols`.
  - [x] `digest_drilldown_navigates_to_listeners_with_public_chip`.
  - [x] `digest_empty_state_strings_present`.
  - [x] `default_view_is_overview`.

### C. Web UI integration

- [x] PLAN-15c (#16) is the canonical place this lands; this plan only ensures the digest view-model is **available** and **JSON-serializable** so the Web crate consumes it via a new `GET /api/digest` endpoint.
- [x] Add `GET /api/digest` to `crates/lazyadmin-web/src/lib.rs` returning the `Digest` struct serialized to JSON. Reuse the existing snapshot polling cache.
- [x] Cross-link: PLAN-15c's "default route renders the digest" task points back here.

### D. Onboarding hint

- [x] On first launch (heuristic: `~/.local/state/lazyadmin/seen-overview.flag` absent), the TUI shows a one-line dim hint above the digest:
  - `New layout: this is the digest. Press [v] for the full Listeners table.`
- [x] After any input is received in `Overview`, write the flag and never show again.

### E. CLI parity

- [x] Add `lazyadmin overview` subcommand that prints the digest as text (mirrors what the TUI shows). `lazyadmin overview --json` emits the `Digest` struct.
- [x] Plumbing: reuse `build_digest` directly; no new logic.
- [x] AGENTS.md validation list gets `cargo run -p lazyadmin-cli -- overview --json` added.

## Acceptance criteria (mirrors #14)

- [x] `lazyadmin tui` cold launch defaults to the digest, not a 1,749-row table.
- [x] `lazyadmin web` cold load default route renders the digest.
- [x] All four sections present with affirmative empty states when applicable.
- [x] `[view all N →]` deep-links to the existing filtered view in both UIs.
- [x] `Everything` / `All` reachable via `:` palette → `view all`, via `/` search, or via `--view everything`.
- [x] Snapshot polling and event hint behavior unchanged.
- [x] JSON snapshot contract unchanged (only **additive** `/api/digest` endpoint).
- [x] `cargo test --workspace` green.
- [x] `cargo run -p lazyadmin-cli -- export --json` byte-identical to the pre-#14 golden.

## Out of scope

- Visual styling of rows (#18).
- Inspector content for digest rows (#17).
- Doctor noise-collapse logic (#15 — but the triage line consumes #15's output once available).

## Implementation notes

- The digest's "Your projects" `last_seen` is best derived from the most recent `Provenance.observed_at` across the project's listeners. If that's not currently exposed, surface it as a method on `ProjectId` joining over `snapshot.listeners` rather than adding a new field to `Project`.
- `ExposedRow.owner_label` should reuse the existing label helper from `lazyadmin-tui`'s row formatter — extract it into `lazyadmin-runtime::view_model::format::owner_label(&Owner) -> String` so TUI and digest stay consistent.
- For the dedupe-by-`owner_pid` rule, only fold rows that share *exposure class* (don't fold a public port into a LAN port row).

## Dogfood-evidence reproduction

After landing, capture a fresh dogfood TUI run; archive under `dogfood-tui-output/` with a screenshot of the new default view. Compare 1:1 with `dogfood-tui-output/lazyadmin-tui-20260429-023602/evidence/screens/00-baseline.png` for the issue close-out.
