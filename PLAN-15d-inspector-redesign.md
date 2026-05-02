# PLAN-15d: Inspector Pane — Richer Than the Row, Not a Transposed Copy (Issue #17)

Parent: [PLAN-15](./PLAN-15-ux-overhaul.md) · Issue: [#17](https://github.com/shuv1337/lazyadmin/issues/17)

The inspector is the only place where the user has committed to one entity. It must be the *richest* surface in the UI — per-entity-kind layouts, no `-` rows, full identifiers (no truncation), related entities listed, action commands previewed before confirmation.

## Prerequisites

- [ ] PLAN-15 Phase 0 complete (`view_model` module exists, classifier registry exists).
- [ ] PLAN-15a's `Digest` view-model exists (some inspector views reuse the same formatting helpers).

## Design contract

The inspector is rendered from a single typed view-model. **No** entity-kind branches in the renderer beyond a top-level dispatch. The TUI and the Web UI both consume the same struct.

```rust
// crates/lazyadmin-runtime/src/view_model/inspector.rs
pub enum InspectorView {
    Listener(ListenerInspector),
    Workload(WorkloadInspector),
    Process(ProcessInspector),
    Project(ProjectInspector),
    Manager(ManagerInspector),
    TrackedRun(TrackedRunInspector),
    WarningGroup(WarningGroupInspector),
}
```

Each variant contains zero `-` strings. Missing data is either omitted or rendered as a single section-level dim message. Identifiers (listener IDs, paths, cmdlines) are the **full** values — not truncated.

## Tasks

### A. Per-entity view-models

- [x] `ListenerInspector`:
  ```rust
  pub struct ListenerInspector {
      pub identity: ListenerIdentity, // listener_id, bind, owner_label, user
      pub process: Option<ProcessFragment>, // parent + children (count + first 3)
      pub related_listeners: Vec<RelatedListener>, // other listeners with same owner_pid
      pub project: Option<ProjectRef>,
      pub confidence: ConfidenceBlock, // value + per-signal explanation
      pub actions: Vec<ActionPreview>, // command preview strings
      pub warnings: Vec<WarningRef>,
  }
  ```
- [x] `ProcessInspector`:
  - parent + children fragment (with one-key jump to the full Process Tree).
  - listeners owned by this process (from `Workload.listeners` / `Listener.owners`).
  - cmdline (full), cwd, exe, user, lazyadmin run id (if present).
  - confidence block.
  - actions.
- [x] `WorkloadInspector`, `ProjectInspector`, `ManagerInspector`, `TrackedRunInspector` — analogous shapes; document the `RELATED` block content for each:
  - Workload: child processes + listeners.
  - Project: workloads + listeners + markers.
  - Manager: managed workloads + adapter health.
  - TrackedRun: tag, command, cwd, state, workload link.
- [x] `WarningGroupInspector` (consumed by PLAN-15b):
  - registry entry (label, remediation).
  - sample entities (up to 10).
  - aggregate count + max severity.
- [x] `InspectorView::lookup(snapshot, kind, id)` — pure projection (named differently from the spec, but same contract; `build_inspector(snapshot, target)` would be a thin wrapper if a future caller wants `EntityRef`-keyed lookup).
- [x] Tests:
  - [x] `inspector_listener_lists_related_listeners_owned_by_same_pid`.
  - [x] `inspector_process_lists_listeners_held_by_pid`.
  - [x] `inspector_no_dash_rows_in_any_variant`.
  - [x] `inspector_listener_id_is_not_truncated_in_view_model`.
  - [x] `inspector_confidence_explains_which_signal_is_best_effort`.

### B. Action command preview

- [x] `ActionPreview { verb, key_hint, command_string, enabled, disabled_reason }` — `disabled` when not applicable (e.g. `restart` on a direct/unmanaged process). `disabled_reason` is shown as a dim explanation, not silently no-op.
- [ ] Wire `ActionPreview::command_string` to the live `lazyadmin-core::actions` dry-run output (currently the previews are hand-written per kind). Tracked as a follow-up: when the TUI rewrite (Task D) lands, we can call `render_dry_run` for the matching `Action` and use its first line as the command_string.
- [x] Tests:
  - [x] `restart_disabled_for_direct_process_with_explicit_reason`.
  - [x] `command_preview_string_matches_expected_form` (unit-level proxy for the dry-run match; the dry-run pairing is tracked alongside the TUI rewrite).

### C. Confidence block

- [x] Replace the opaque `confidence best-effort` line. The block contains:
  - `value: Confidence` (existing enum).
  - `signals: Vec<ConfidenceSignalEntry { signal: ConfidenceSignal, adapter, claim }>` derived by classifying the existing `Provenance.adapter` strings into one of `ProcfsPidInode | ContainerInspect | CgroupCorrelation | ManagerAttribution | TrackedRunRegistry | PortlessRoutes | BestEffort`. Unrecognized adapters fall through to `BestEffort` so the user sees the truthful label rather than a confident wrong one.
- [x] *No* changes to `Provenance` were needed — we classify the existing adapter string. Snapshot JSON stays bit-identical to PLAN-14, so the byte-stability check is satisfied by construction (no schema change touched). If a future signal class requires more metadata than the adapter name, extend `Provenance` additively then.
- [x] Tests:
  - [x] `inspector_confidence_explains_which_signal_is_best_effort` covers the signal-classification contract.

### D. TUI rendering

*Status: landed. The TUI now builds entity inspectors through `lazyadmin_runtime::view_model::InspectorView::lookup(...)`, projects `to_sections()` into `InspectorSectionVm`, wraps section rows without truncating values, supports numeric jump shortcuts for related entities, exposes `[v] view all related` overflow into a filtered Listeners table, and opens action-preview confirmation modals with the command preview in the modal header/body.*

- [x] Replace `render_inspector` (~line 3218 in `crates/lazyadmin-tui/src/lib.rs`) with a top-level dispatch on `InspectorView::to_sections()`. Each section: `IDENTITY`, `PROCESS`, `RELATED`, `PROJECT`, `CONFIDENCE`, `ACTIONS`, `WARNINGS`. Skip absent sections instead of rendering `-`.
- [x] No `Listener id   tcp:127.0.0…` ellipsis. The inspector pane wraps long values across lines (Ratatui `Paragraph::wrap`) but never truncates the value itself. Tests assert full-string presence at 38/60/120-col widths.
- [x] `RELATED` block — one-key jump:
  - [x] `[1]…[9]` selects the related listener from the inspector pane.
  - [x] More than 9 → `[v] view all related` opens a filtered Listeners table.
- [x] `ACTIONS` block:
  - [x] Each action line: `[r] restart    systemctl --user restart workerd@17659.scope`.
  - [x] Disabled actions render dim with `[r] restart — disabled (direct process)`.
  - [x] Pressing the key opens the typed-verb confirmation modal *with the command preview already echoed in the modal header*.
- [x] Modal hint inside the modal, not in the footer (cross-link to PLAN-15 #20).
- [x] Tests:
  - [x] `inspector_lists_related_listeners_at_38_col_width`.
  - [x] `inspector_listener_id_full_string_present_at_38_col_width`.
  - [x] `inspector_disabled_action_renders_with_reason`.
  - [x] `inspector_no_dash_rows_visible`.
  - [x] `inspector_full_cmdline_wraps_without_truncation`.
  - [x] `inspector_jump_shortcut_selects_related_listener`.
  - [x] `inspector_view_all_related_filters_listener_table`.
  - [x] `inspector_action_key_opens_confirmation_with_command_preview`.
  - [x] `inspector_disabled_action_key_shows_reason_without_confirmation`.

### E. Web UI rendering (#16)

- [x] PLAN-15c consumes `InspectorView` from `GET /api/inspector?kind=…&id=…`. The Web UI now renders the **same per-kind sections** the TUI will render once Task D lands — they share `InspectorView::to_sections()` semantics (per-kind in JSON, flattened to `{ heading, rows: [{label, value, secondary?, jump_target?}] }` on the JS side; the JS mirror is a 1:1 port of the Rust `to_sections()`). Section headings (`IDENTITY`, `PROCESS`, `RELATED`, `PROJECT`, `CONFIDENCE`, `ACTIONS`, `WARNINGS`, etc.) are the same on both surfaces.
- [x] `show raw` toggle on each Web inspector reveals the underlying snapshot fragment. Toggle state per-session.
- [x] No `<pre>{JSON.stringify(x, null, 2)}</pre>` in visible UI. The "raw" toggle counts; it's an *opt-in* debugging surface (still asserted by `index_html_does_not_contain_pre_json_dump`).
- [x] `inspector_route_serves_per_kind_typed_shape` test confirms the API returns the new typed shape (`identity` field present, no legacy `facts` field, `identity.listener_id` echoes the full id without truncation).

### F. Polish surface (#22 cosmetics that depend on this issue)

- [x] Action labels become labeled keybind hints + state (replaces the lowercase tag list `open  logs  restart  stop  free-port`). Lands here structurally; #22 picks up any narrow-pane wording polish that remains.

## Acceptance criteria (mirrors #17)

- [x] Per-entity-kind inspector layouts: Listener, Workload, Process, Project, Manager, TrackedRun, WarningGroup.
- [x] No `-` or `unavailable` rows. Asserted by `inspector_no_dash_rows_in_any_variant`.
- [x] Listener ID and similar identifiers shown in full. Asserted by `inspector_listener_id_is_not_truncated_in_view_model`.
- [x] `RELATED` block lists co-owned listeners (or analogous) with `jump_target` metadata. TUI one-key shortcut wiring lands with Task D.
- [x] `CONFIDENCE` block explains which signal was best-effort.
- [x] Action lines preview the exact command lazyadmin will run, before the typed-verb confirmation. (Hand-written previews today; pairing with `render_dry_run` is tracked alongside Task D.)
- [x] Action affordances labeled with keybind + enabled/disabled state.
- [x] Web UI inspector uses the same per-entity-kind layouts (rendered HTML, not `<pre>`).
- [x] Tests: no-`-`-rows, related-listeners block, action-preview command strings.

## Out of scope

- New mutating actions.
- New entity kinds.
- Theme work.

## Implementation notes

- The "related listeners owned by the same `owner_pid`" join is already in the snapshot graph via `Listener.owners` + `Workload.listeners`. No adapter work needed.
- `ConfidenceSignal` enum starts small: `ProcfsPidInode | CgroupCorrelation | ManagerAttribution | TrackedRunRegistry | ContainerInspect | PortlessRoutes`. Extend as needed; classification stays in `lazyadmin-core::correlate`.
- The renderer must never call `truncate(width)` on any field within a section's value column. Wrapping is fine; truncation isn't.
- Reuse the row-formatter helpers from PLAN-15a (`format::owner_label`, etc.) for the section bodies so wording stays consistent with the digest.

## Dogfood-evidence reproduction

After landing, compare against:

- `dogfood-tui-output/lazyadmin-tui-20260429-023602/evidence/screens/00-baseline.txt` — `tcp:127.0.0…` ellipsis must be gone.
- `evidence/screens/11-view-public.txt` — Provenance ellipsis gone.
- `evidence/screens/18-view-process-tree.txt` — process inspector's four `-` rows gone.

## Risk

| Risk | Mitigation |
| ---- | ---------- |
| Inspector fields creep — every release adds a row. | View-model owns the canonical schema; PRs that add fields update the registry + tests. |
| Confidence signal classification is fuzzy. | Start with the small enum above; only extend when an emitter has a *new* signal class to surface. |
| `Provenance` additive change breaks downstream consumers. | Mark new fields `#[serde(default, skip_serializing_if = "…")]`; round-trip byte-stable against existing goldens before merge. |
