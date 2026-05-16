# TUI keybindings

Default bindings match v0.1 (`q`, `ctrl+c`, `/`, `:`, `tab`, `shift+tab`, `enter`, `l`, `p`, `t`, `m`, `r`, `s`, `f`, `k`, `o`, `e`, `y`, `S`, `R`, `?`, `[`, `]`, `>`).

Inline overrides:

```toml
[ui.keybindings.overrides]
quit = "Q"
free_port = "ctrl+f"
open = "o"
sort_next = "n"
sort_prev = "N"
sort_toggle = "."
```

Or a file:

```toml
[ui.keybindings]
path = "~/.config/lazyadmin/keybindings.toml"
```

File format:

```toml
inherit = "default"

[overrides]
quit = "Q"
```

`lazyadmin config check --json` validates duplicate bindings and unknown action names with suggestions.

### Search mode

The TUI starts with global search focused. These bindings apply while the search
input is active:

| Key | Purpose |
|-----|---------|
| `/` | Re-focus global search from normal mode without clearing the query |
| text / digits | Search across listeners, processes, workloads, projects, managers, and views |
| `enter` | Open the highlighted result |
| `esc` | Clear the query, blur to rows, and restore the previous view |
| `backspace` | Delete the previous query character |
| `up` / `down` | Move result selection |
| `pageup` / `pagedown` | Move result selection by a page |
| `home` / `end` | Jump to first or last result |
| `tab` / `shift+tab` | Move pane focus |

`filter` and `toggle_filter` remain valid keybinding action names for backward
compatibility, but both resolve to the global `search` command.

The command palette remains on `:` in the TUI. In the Web UI, `/` focuses global
search and `Ctrl+K` / `Cmd+K` opens the palette.

### Sort actions

| Action name | Default | Purpose |
|-------------|---------|---------|
| `sort_next` | `]` | Next sortable listener column |
| `sort_prev` | `[` | Previous sortable listener column |
| `sort_toggle` | `>` | Toggle ascending/descending on active sort column |
