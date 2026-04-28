# lazyadmin TUI

The TUI is a Ratatui projection of the core snapshot model. It does not own runtime/correlation logic; interactive state is rebuilt from authoritative core snapshots.

Run:

```bash
lazyadmin
lazyadmin tui --headless --json
```

Layouts switch at 100, 80, 60, and below-60 columns. Below 60 columns the TUI refuses and suggests CLI commands.

Views include Everything, Ports, Public, Conflicts, Projects, Managers, Orphans, Tracked Runs, Logs, Doctor, Process Tree (`t`), and Metrics (`m`). Live procfs, Docker-compatible container, and systemd D-Bus discovery events are treated as refresh hints and periodic snapshot polling remains authoritative.

Use `?` for a help overlay sourced from active keybindings and `:` for the command palette surface. The palette supports Process Tree, Metrics, theme switching, and config reload commands. `y` copies a redacted diagnostic to the clipboard and falls back to `$XDG_STATE_HOME/lazyadmin/copies/<timestamp>.md`; `o` opens only loopback listeners on common HTTP ports unless `actions.open_non_loopback = true`.

Search (`/`) filters rendered rows and process-tree labels. `S` toggles system-service visibility by rebuilding the rendered model from the retained snapshot. In Process Tree, `t` opens the tree and pressing `t` on a node toggles expand/collapse while preserving the selected `ProcessKey` across refreshes.
