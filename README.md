# lazyadmin

`lazyadmin` is a Linux-first local runtime control plane for developers and coding agents. It discovers ports, processes, containers, systemd units, projects, tracked runs, logs, and safe actions through a Rust core, CLI, and Ratatui TUI.

## Install

```bash
cargo install --path crates/lazyadmin-cli --locked
# or from a future release:
cargo install lazyadmin
```

No daemon or root install is required.

## Quick checks

```bash
lazyadmin doctor
lazyadmin doctor --json
lazyadmin ps --json
lazyadmin public --json
lazyadmin conflicts --json
```

## Explain a port

```bash
lazyadmin :3000
lazyadmin :3000 --json
lazyadmin "tcp/[::1]:3000"
```

Point queries bypass the default two-tier system-service filter.

## Tracked runs

Wrap long-running dev servers so they can be found and stopped later:

```bash
lazyadmin run --tag my-web --detach -- npm run dev
lazyadmin runs --json
lazyadmin run stop tag:my-web
```

See [`docs/tracked-run-spawn-decision.md`](docs/tracked-run-spawn-decision.md).

## Free a port safely

```bash
lazyadmin export --json > /tmp/before.json
lazyadmin free 3000
lazyadmin diff /tmp/before.json - --json
```

`free` uses manager-aware planning and confirmation rather than unsafe first-line kill commands. See [`docs/action-safety.md`](docs/action-safety.md).

## TUI

Run `lazyadmin` with no args to launch the Ratatui TUI, or use `lazyadmin tui --headless --json` for automation/debugging without entering raw terminal mode. Responsive behavior:

- `>=100` columns: Groups, Workloads/Listeners, Inspector panes
- `80–99` columns: inspector tab
- `60–79` columns: single pane with view switching
- `<60` columns: refuses with CLI hints

Keybindings include `/`, `:`, `Tab`, `Enter`, `l`, `p`, `t`, `m`, `r`, `s`, `f`, `k`, `o`, `e`, `y`, `S`, `R`, `?`, and `q`, and interactive dispatch honors configured overrides. Themes and keybindings are configurable; see [`docs/tui.md`](docs/tui.md), [`docs/themes.md`](docs/themes.md), and [`docs/keybindings.md`](docs/keybindings.md). v0.2 TUI copy/open/palette actions are still limited; prefer CLI commands for external or destructive operations.

## Agent skill

The coding-agent skill ships in [`skills/lazyadmin-agent/`](skills/lazyadmin-agent/). Build the release tarball with:

```bash
scripts/build-skill-tarball.sh
```

Agents should check `command -v lazyadmin && lazyadmin doctor --json` once per session, prefer JSON, wrap long-running commands with `lazyadmin run --tag ... --detach -- <cmd>`, avoid unsafe kill patterns, capture diffs around mutations, and stop their own tagged runs.

## Docs

- [`docs/spec.md`](docs/spec.md)
- [`docs/adapter-protocol.md`](docs/adapter-protocol.md)
- [`docs/action-safety.md`](docs/action-safety.md)
- [`docs/tui.md`](docs/tui.md)
- [`docs/themes.md`](docs/themes.md)
- [`docs/keybindings.md`](docs/keybindings.md)
- [`docs/troubleshooting.md`](docs/troubleshooting.md)
- [`docs/agent-integration.md`](docs/agent-integration.md)
- [`docs/schema/snapshot-v1.md`](docs/schema/snapshot-v1.md)
- [`docs/schema/diff-v1.md`](docs/schema/diff-v1.md)
- [`docs/schema/doctor-v1.md`](docs/schema/doctor-v1.md)

## Scope

v0.1 is Linux-first. v0.2 work is in progress: TUI rendering/configuration is substantially improved, while full adapter-native event streams and some interactive action plumbing remain deferred.
