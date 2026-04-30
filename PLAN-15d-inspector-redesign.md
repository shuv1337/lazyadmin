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

- [ ] `ListenerInspector`:
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
- [ ] `ProcessInspector`:
  - parent + children fragment (with one-key jump to the full Process Tree).
  - listeners owned by this process (from `Workload.listeners` / `Listener.owners`).
  - cmdline (full), cwd, exe, user, lazyadmin run id (if present).
  - confidence block.
  - actions.
- [ ] `WorkloadInspector`, `ProjectInspector`, `ManagerInspector`, `TrackedRunInspector` — analogous shapes; document the `RELATED` block content for each:
  - Workload: child processes + listeners.
  - Project: workloads + listeners + last-seen.
  - Manager: managed workloads + adapter health.
  - TrackedRun: pid (if alive), workload link, log path.
- [ ] `WarningGroupInspector` (consumed by PLAN-15b):
  - registry entry (label, remediation).
  - sample entities (up to 10).
  - aggregate count + first/last seen.
- [ ] `pub fn build_inspector(snapshot: &Snapshot, target: EntityRef) -> Option<InspectorView>` — pure projection.
- [ ] Tests:
  - [ ] `inspector_listener_lists_related_listeners_owned_by_same_pid`.
  - [ ] `inspector_process_lists_listeners_held_by_pid`.
  - [ ] `inspector_no_dash_rows_in_any_variant`.
  - [ ] `inspector_listener_id_is_not_truncated_in_view_model`.
  - [ ] `inspector_confidence_explains_which_signal_is_best_effort`.

### B. Action command preview

- [ ] Reuse the existing dry-run output from `lazyadmin-core::actions`. Surface that string ahead of the confirmation modal instead of inside it.
- [ ] `ActionPreview { verb, key_hint, command_string, enabled, disabled_reason }` — `disabled` when not applicable (e.g. `restart` on a direct/unmanaged process). `disabled_reason` is shown as a dim explanation, not silently no-op.
- [ ] Tests:
  - [ ] `restart_disabled_for_direct_process_with_explicit_reason`.
  - [ ] `command_preview_string_matches_dry_run_output`.

### C. Confidence block

- [ ] Replace the opaque `confidence best-effort` line. The block contains:
  - `value: Confidence` (existing enum).
  - `signals: Vec<ConfidenceSignal>` — each describing which signal contributed (procfs PID→inode link direct, cgroup correlation skipped, manager attribution heuristic, etc.).
- [ ] If we need extra signal metadata not in the snapshot today, **extend `Provenance` additively** (new optional fields, default-skipped-when-empty in serialize so JSON contract stays backwards-compatible). Verify byte stability of `cargo run -p lazyadmin-cli -- export --json` against the empty-snapshot golden.
- [ ] Tests:
  - [ ] `confidence_block_lists_each_signal_class`.
  - [ ] `provenance_additive_fields_default_to_none_and_round_trip_byte_stable`.

### D. TUI rendering

- [ ] Replace `render_inspector` (~line 2478 in `crates/lazyadmin-tui/src/lib.rs`) with a top-level dispatch on `InspectorView`. Each variant renders sections: `IDENTITY`, `PROCESS`, `RELATED`, `PROJECT`, `CONFIDENCE`, `ACTIONS`, `WARNINGS`. Skip absent sections instead of rendering `-`.
- [ ] No `Listener id   tcp:127.0.0…` ellipsis. The inspector pane wraps long values across lines (Ratatui `Paragraph::wrap`) but never truncates the value itself. Tests assert full-string presence at 38/60/120-col widths.
- [ ] `RELATED` block — one-key jump:
  - `[1]…[9]` selects the related listener; `Enter` jumps to its inspector.
  - More than 9 → `[v] view all related` opens a filtered Listeners table.
- [ ] `ACTIONS` block:
  - Each action line: `[r] restart    systemctl --user restart workerd@17659.scope`.
  - Disabled actions render dim with `[r] restart — disabled (direct process)`.
  - Pressing the key opens the typed-verb confirmation modal *with the command preview already echoed in the modal header*.
- [ ] Modal hint inside the modal, not in the footer (cross-link to PLAN-15 #20).
- [ ] Tests:
  - [ ] `inspector_lists_related_listeners_at_38_col_width`.
  - [ ] `inspector_listener_id_full_string_present_at_38_col_width`.
  - [ ] `inspector_disabled_action_renders_with_reason`.
  - [ ] `inspector_no_dash_rows_visible`.

### E. Web UI rendering (#16)

- [ ] PLAN-15c consumes `InspectorView` from `GET /api/inspector?kind=…&id=…`. This plan asserts the JSON shape is stable and that the Web UI's templated layout matches the TUI's sections 1:1 (same headings, same wording).
- [ ] `Show raw` toggle on each Web inspector reveals the underlying snapshot fragment. Toggle state per-session.
- [ ] No `<pre>{JSON.stringify(x, null, 2)}</pre>` in visible UI. The "raw" toggle counts; it's an *opt-in* debugging surface.

### F. Polish surface (#22 cosmetics that depend on this issue)

- [ ] Action labels become labeled keybind hints + state (replaces the lowercase tag list `open  logs  restart  stop  free-port`). Lands here structurally; #22 picks up any narrow-pane wording polish that remains.

## Acceptance criteria (mirrors #17)

- [ ] Per-entity-kind inspector layouts: Listener, Workload, Process, Project, Manager, TrackedRun, WarningGroup.
- [ ] No `-` or `unavailable` rows.
- [ ] Listener ID and similar identifiers shown in full.
- [ ] `RELATED` block lists co-owned listeners (or analogous) with one-key jump.
- [ ] `CONFIDENCE` block explains which signal was best-effort.
- [ ] Action lines preview the exact command lazyadmin will run, before the typed-verb confirmation.
- [ ] Action affordances labeled with keybind + enabled/disabled state.
- [ ] Web UI inspector uses the same per-entity-kind layouts (rendered HTML, not `<pre>`).
- [ ] Tests: no-`-`-rows, related-listeners block, action-preview command strings.

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
