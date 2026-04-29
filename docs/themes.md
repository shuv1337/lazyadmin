# TUI themes

Built-ins: `default-dark` (Night Owl), `night-owl` (alias of `default-dark`), `default-light` (Night Owl Light), `night-owl-light` (alias of `default-light`), `high-contrast`, `solarized-dark`.

The default dark and light themes ship Sarah Drasner's [Night Owl](https://github.com/sdras/night-owl-vscode-theme) palette so the lazyadmin TUI matches the editor/terminal theme used elsewhere in the workspace.

Configure either a built-in name or a TOML path:

```toml
[ui.theme]
name = "high-contrast"
# path = "/home/me/.config/lazyadmin/themes/my-theme.toml"
```

Theme color values accept named ANSI colors such as `red`, `green`, `bright-blue`, or hex colors `#RRGGBB` / `#RRGGBBAA` (alpha ignored). Each theme declares `fallback_palette` (`sixteen`, `two_fifty_six`, or `truecolor` in serialized Rust form). `lazyadmin config check` validates invalid built-in names, missing/invalid explicit theme files, TOML parse errors, and color strings.

Custom themes can be loaded by absolute `path` or by `name` from `$XDG_CONFIG_HOME/lazyadmin/themes/<name>.toml` (falling back to `~/.config`). Theme files may omit surface keys; missing values inherit from `default-dark`. At runtime lazyadmin detects terminal color support from environment hints and downgrades to the theme fallback palette when needed.
