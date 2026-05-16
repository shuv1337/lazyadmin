# lazyadmin TUI

The TUI is a Ratatui projection of the core snapshot model. It does not own runtime/correlation logic; interactive state is rebuilt from authoritative core snapshots.

Run:

```bash
lazyadmin
lazyadmin tui --headless --json
```

On launch, `lazyadmin` keeps the Overview digest as the visible body but puts
keyboard focus in global Search mode. Typing immediately switches to the Search
view. `Esc` clears the query, blurs to the rows pane, and restores the previous
view; `/` focuses search again without clearing the stored query.

Layouts switch at 100, 80, 60, and below-60 columns. Below 60 columns the TUI refuses and suggests CLI commands.

Views include Everything, Ports, Public, Conflicts, Projects, Managers, Orphans, Tracked Runs, Logs, Doctor, Process Tree (`t`), and Metrics (`m`). Live procfs, Docker-compatible container, and systemd D-Bus discovery events are treated as refresh hints and periodic snapshot polling remains authoritative.

The default TUI presentation favors a high-contrast operations console over raw debug output: a compact header summarizes listener and adapter state, the view rail uses muted inactive labels with an accented active marker, the main table uses compact owner/runtime labels instead of raw `ProcessKey` dumps, and the inspector renders key fields plus provenance as a readable detail pane. `high-contrast` remains available for limited-color terminals and accessibility checks.

Use `?` for a help overlay sourced from active keybindings and `:` for the command palette surface. The palette supports Process Tree, Metrics, theme switching, and config reload commands. `y` copies a redacted diagnostic to the clipboard and falls back to `$XDG_STATE_HOME/lazyadmin/copies/<timestamp>.md`; `o` opens only loopback listeners on common HTTP ports unless `actions.open_non_loopback = true`.

Global search (`/`) searches listeners, processes, workloads, projects,
managers, and rail views through the shared `lazyadmin.search.v1` projection.
Existing per-view filters still read the same query for Workloads, Doctor, and
other local projections as a v1 compatibility escape hatch, so global results
and page-local filtering can coexist until that older filter surface is cleaned
up. `S` toggles system-service visibility by rebuilding the rendered model from
the retained snapshot. In Process Tree, `t` opens the tree and pressing `t` on a
node toggles expand/collapse while preserving the selected `ProcessKey` across
refreshes.

### Listener sorting

The listener table can be sorted by column in both TUI and Web UI:

- `]` moves to the next sortable listener column.
- `[` moves to the previous sortable listener column.
- `>` toggles ascending/descending for the active sort column.
- Active sort is shown in the table header with `▲`/`▼`.
- Sort commands preserve selected listener identity where possible.

In the Web UI, click (or keyboard-focus and press Enter/Space on) any listener column header to change sort. The active sort is preserved in the URL hash. Switching to a different column always resets direction to ascending. The Web UI exposes `Port`, `Bind`, `Exposure`, `Owner`, `Project`, `Confidence`, and `Warnings` as sortable columns; the TUI exposes `Port`, `Bind`, `Owner`, `Runtime`, and `Scope`.
