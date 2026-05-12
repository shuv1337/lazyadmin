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

### Sort actions

| Action name | Default | Purpose |
|-------------|---------|---------|
| `sort_next` | `]` | Next sortable listener column |
| `sort_prev` | `[` | Previous sortable listener column |
| `sort_toggle` | `>` | Toggle ascending/descending on active sort column |
