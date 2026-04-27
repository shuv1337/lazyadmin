# lazyadmin TUI

The TUI is a Ratatui projection of the core snapshot model. It does not own runtime/correlation logic; it renders core view-models and calls the same action planning paths as the CLI.

Run:

```bash
lazyadmin
lazyadmin tui --headless --json
```

Layouts switch at 100, 80, 60, and below-60 columns. Below 60 columns the TUI refuses and suggests CLI commands.

Views include Everything, Ports, Public, Conflicts, Projects, Managers, Orphans, Tracked Runs, Logs, Doctor, Process Tree (`t`), and Metrics (`m`). Live discovery events are treated as refresh hints; periodic snapshot polling remains authoritative for container and systemd state in v0.2.

Use `?` for help and `:` for the command palette. Palette entries include `process-tree`, `metrics`, `theme <name>`, and `reload`.
