# TUI themes

Built-ins: `default-dark`, `default-light`, `high-contrast`, `solarized-dark`.

Configure either a built-in name or a TOML path:

```toml
[ui.theme]
name = "high-contrast"
# path = "/home/me/.config/lazyadmin/themes/my-theme.toml"
```

Theme color values accept named ANSI colors such as `red`, `green`, `bright-blue`, or hex colors `#RRGGBB` / `#RRGGBBAA` (alpha ignored). Each theme declares `fallback_palette` (`sixteen`, `two_fifty_six`, or `truecolor` in serialized Rust form). `lazyadmin config check` validates invalid built-in names, missing/invalid explicit theme files, TOML parse errors, and color strings. XDG theme-name lookup and missing-key inheritance are not shipped in v0.2; use an explicit `path` for custom themes.
