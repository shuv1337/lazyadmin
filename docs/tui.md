# lazyadmin TUI

The TUI is a Ratatui projection of the core snapshot model. It does not own runtime/correlation logic; interactive state is rebuilt from authoritative core snapshots.

Run:

```bash
lazyadmin
lazyadmin tui --headless --json
```

Layouts switch at 100, 80, 60, and below-60 columns. Below 60 columns the TUI refuses and suggests CLI commands.

Views include Everything, Ports, Public, Conflicts, Projects, Managers, Orphans, Tracked Runs, Logs, Doctor, Process Tree (`t`), and Metrics (`m`). Live procfs discovery events are treated as refresh hints and periodic snapshot polling remains authoritative. Native container/systemd adapter event streams are deferred, so their changes appear on the next snapshot tick.

Use `?` for a help overlay sourced from active keybindings and `:` for the command palette surface. In v0.2, palette/copy/open entries are visible/testable helpers but not a complete interactive action system; prefer CLI commands for destructive or external actions.

Search (`/`) filters rendered rows and process-tree labels. `S` toggles system-service visibility by rebuilding the rendered model from the retained snapshot.
