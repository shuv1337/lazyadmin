# PLAN-15b: Doctor — Aggregate Warnings by `(code, severity)` and Rank by Actionability (Issue #15)

Parent: [PLAN-15](./PLAN-15-ux-overhaul.md) · Issue: [#15](https://github.com/shuv1337/lazyadmin/issues/15)

Aggregate the ~2,183 individual `fd_permission_denied` rows (and friends) into ranked, actionable groups. Default-collapse noise. Lead the header with *actionability*, not raw counts.

## Prerequisites

- [x] Phase 0 of PLAN-15 complete enough to start grouping:
  - [x] `lazyadmin-runtime::view_model` module exists.
  - [x] Warning code → tier/remediation registry landed in `lazyadmin-core::doctor`.

## Hard constraint

`lazyadmin doctor --json` and `lazyadmin export --json` continue to emit the **flat list** of `Warning` entries. Grouping is a *presentation* transform only. Verify with a byte-stable golden before and after.

## Tasks

### A. Core registry (Phase 0.2 deliverable, re-asserted)

- [x] Inventory every warning code currently emitted:
  - [x] `rg "Warning \{ code:" crates/lazyadmin-* | sort -u` — list all literals.
  - [x] `rg "WarningCode::|warning_code\(" crates/lazyadmin-*` — list any constants.
- [x] Land `WarningCodeMeta` registry in `crates/lazyadmin-core/src/doctor/registry.rs` (or `doctor.rs` if we keep it flat) with one row per shipped code:
  - `code: &'static str`
  - `tier: WarningTier` (`Critical | Actionable | Noise`)
  - `label: &'static str` — short human label without truncation, e.g. `Port conflict`, `Permission denied (fd)`.
  - `remediation: &'static str` — one-line "what to do", e.g. `Run portless prune`, `Run with sudo to read`.
- [x] Public API:
  ```rust
  pub fn classify(code: &str) -> WarningCodeMeta;
  pub const ALL_CODES: &[WarningCodeMeta];
  ```
- [x] Unknown code default: `tier: Actionable, label: code, remediation: "inspect details"`.
- [x] Tests:
  - [x] `every_emitted_code_has_registry_entry` — compile-time emitted-code inventory list is asserted against the registry.
  - [x] `unknown_code_defaults_to_actionable`.
  - [x] `classify_is_pure` (returns the same metadata for repeated calls).

### B. WarningGroup view-model

- [x] In `crates/lazyadmin-runtime/src/view_model/doctor_groups.rs`:
  ```rust
  pub struct WarningGroup {
      pub code: String,
      pub severity: WarningSeverity,
      pub tier: WarningTier,
      pub label: String,
      pub remediation: String,
      pub count: usize,
      pub sample_entities: Vec<EntityRef>, // up to 5
      pub expanded: bool,                  // default per tier
  }

  pub struct DoctorGroupsView {
      pub groups: Vec<WarningGroup>,        // sorted: critical > actionable > noise; within tier, count desc, code asc
      pub actionable_count: usize,          // sum of group.count for tier in {Critical, Actionable}
      pub noise_group_count: usize,
      pub noise_total_count: usize,
      pub last_check: chrono::DateTime<Utc>,
  }
  ```
- [x] `pub fn build_doctor_groups(snapshot: &Snapshot) -> DoctorGroupsView` — pure projection over `snapshot.warnings`, classified via the core registry.
- [x] Default expansion: `Critical` and `Actionable` groups expanded; `Noise` collapsed.
- [x] Tests:
  - [x] `groups_2183_fd_permission_into_one_noise_row`.
  - [x] `critical_port_conflict_appears_above_actionable_warnings`.
  - [x] `sample_entities_capped_at_five_and_stable`.
  - [x] `affirmative_empty_state_when_no_actionable_warnings` — the resulting view is consumed by the renderer and copy starts `Everything's clean`.

### C. TUI integration

- [x] `render_doctor_view` in `crates/lazyadmin-tui/src/lib.rs` (~line 2565) rewritten to render grouped doctor rows:
  - [x] Header counter leads with actionable/noise counts.
  - [x] Empty state copy starts `Everything's clean`.
  - [x] Each row layout:
    ```
    ⚠  <SEV>  <code>  <one-liner sample>           <count>  [inspect →]
    ```
  - [x] Severity glyph + color from theme (`pip_warn`, `pip_error`, `footer` for noise).
  - Expand/collapse toggle: `Enter` on a group row toggles `expanded`. Expanded groups render their underlying per-warning rows below the header.
- [x] Severity filter chips on the toolbar: `Critical / Warning / Info / All`. Mirrors #19's chip mechanism inside `Listeners`.
- [x] Per-warning copy (the underlying flat row) is rewritten so no column truncates mid-word at 160 cols. Audit current strings (`fd_permission_d`, `permission denied readin`, `inspect details and sour`) and replace with full-word, padded labels. The label/remediation strings come straight from the registry — *never* truncated.
- [x] Inspector pane for a `WarningGroup` is one of #17's per-entity-kind layouts; this plan only asserts the view-model is wired through.
- [x] Tests in `lazyadmin-tui`:
  - [x] `doctor_grouping_collapsed_by_default_for_noise`.
  - [x] `doctor_grouping_expand_renders_individual_rows`.
  - [x] `doctor_no_column_truncates_midword_at_160_cols`.
  - [x] `doctor_affirmative_empty_state_present`.
  - [x] `doctor_severity_filter_chip_changes_count`.

### D. Web UI integration

- [x] PLAN-15c #16 owns the visual rebuild. This plan ensures:
  - [x] New endpoint `GET /api/doctor` returns `DoctorGroupsView` JSON.
  - [x] **Existing** `GET /api/snapshot` is unchanged.
  - [x] `lazyadmin doctor --json` (CLI) is unchanged: still flat warnings list.

### E. CLI parity

- [x] `lazyadmin doctor --groups` flag prints the grouped view as human text (collapse/expand controlled by `--all` / `--actionable`).
- [x] `lazyadmin doctor --json` unchanged. `lazyadmin --json doctor --groups` is additive grouped JSON output.

### F. Digest integration (#14)

- [x] PLAN-15a's `TriageSection` consumes:
  ```rust
  TriageSummary {
      actionable: dgv.actionable_count,
      noise_groups: dgv.noise_group_count,
      noise_total: dgv.noise_total_count,
      last_check: dgv.last_check,
  }
  ```
  Make sure this struct is exported. (`TriageSummary` is exported from `lazyadmin_runtime::view_model`.)

## Acceptance criteria (mirrors #15)

- [x] Doctor view groups warnings by `(code, severity)` with a count column.
- [x] Each group has a one-line "what it means / what to do" sourced from the registry.
- [x] Noise tier groups default-collapse with `×N` and a hint.
- [x] Expand a group to see its individual entities.
- [x] Severity filter chips on the toolbar.
- [x] Header counter changed from raw counts to actionability-led wording.
- [x] Affirmative empty state when zero actionable.
- [x] No column truncates mid-word at 160 cols.
- [x] `lazyadmin export --json` and `lazyadmin doctor --json` continue to emit the flat list (contract test + CLI JSON smoke).
- [x] Tests: grouping logic in core, collapsed/expanded rendering in TUI, affirmative empty state.

## Out of scope

- New warning codes or new doctor checks.
- JSON contract changes.

## Implementation notes

- Tier mapping per #15: Critical → `Actionable`; Warning with a finite remediable target → `Actionable`; `fd_permission_denied`, `scan_skipped`, and similar systemic noise → `Noise`. Encode once, in the registry, as a function of `code`.
- Stable sort keys: within a tier sort by `count desc, code asc`. Tests must pin the order to avoid map-iteration flake.
- `last_check` comes from `snapshot.metadata` (existing `observed_at`-style field); if missing, fall back to `Utc::now()` at view-model build time.

## Dogfood-evidence reproduction

After landing, capture a fresh `Doctor` view screenshot and compare against `dogfood-tui-output/lazyadmin-tui-20260429-023602/evidence/screens/17-view-doctor.txt` and `59-doctor-view.txt` for the issue close-out.
