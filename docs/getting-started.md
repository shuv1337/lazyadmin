# Getting started

## Prerequisites

- Linux host (kernel 5.x or newer recommended)
- Rust toolchain 1.85 or newer
- Optional: Docker Engine API for container discovery
- Optional: systemd for unit and journal integration
- Optional: `ss` (iproute2) as a socket fallback

## Installation

Install from the workspace:

```bash
cd lazyadmin
cargo install --path crates/lazyadmin-cli --locked
```

Verify the installation:

```bash
lazyadmin --version
```

## First run

### Check system health

```bash
lazyadmin doctor
```

This scans all adapters and reports subsystem status. A healthy system
shows all `[OK]` markers. Warnings such as an accessible Docker socket
are expected on developer machines and include actionable hints.

### Export a full snapshot

```bash
lazyadmin export --json > snapshot.json
```

The snapshot includes listeners, workloads, processes, managers,
projects, tracked runs, and warnings with full provenance. Schema:
`lazyadmin.snapshot.v1`.

### Get a high-level digest

```bash
lazyadmin overview --json
```

The digest summarizes exposed listeners, conflicts, your projects, and
triage state in a single JSON object suitable for dashboards or agent
checks.

## Common workflows

### Find what is listening on a port

```bash
lazyadmin :3000
lazyadmin :3000 --json
lazyadmin "tcp/[::1]:3000"
```

Point queries bypass the default two-tier system-service filter and show
the exact owner, runtime, and confidence.

### List all listeners

```bash
lazyadmin ps --json | jq '.listeners | length'
lazyadmin public --json      # only public/LAN-facing
lazyadmin conflicts --json     # only contended ports
```

### Free a port safely

```bash
lazyadmin export --json > /tmp/before.json
lazyadmin free 3000
lazyadmin diff /tmp/before.json - --json
```

`free` resolves the listener, validates the direct process owner, sends
SIGTERM after a single consolidated confirmation, rescans, and reports
the result. SIGKILL is never automatic. See [action-safety.md](action-safety.md)
for danger levels and confirmation policies.

### Track a long-running dev server

```bash
lazyadmin run --tag my-web --detach -- npm run dev
lazyadmin runs --json
lazyadmin run stop tag:my-web
```

Tracked runs are recorded in `/run/user/$UID/lazyadmin/runs` and can be
found and stopped later by tag. They are visible in the TUI and Web UI
under the Tracked Runs view.

### Watch discovery events

```bash
lazyadmin events --once --json
```

Discovery events include procfs heartbeats, container events, and
systemd D-Bus signals. The TUI and Web UI use these as refresh hints
while periodic snapshot polling remains authoritative.

## Using the TUI

Run with no arguments:

```bash
lazyadmin
```

Layouts adapt to terminal width:

| Width | Layout |
|-------|--------|
| >=100 | Three-pane: views, main table, inspector |
| 80–99 | Two-pane: main + inspector tab |
| 60–79 | Single pane with view switching |
| <60 | Refuses with CLI hints |

Key commands:

| Key | Action |
|-----|--------|
| `q` | Quit |
| `/` | Search |
| `:` | Command palette |
| `Tab` / `Shift+Tab` | Next/previous pane |
| `Enter` | Inspect selected row |
| `l` | Listeners view |
| `p` | Projects view |
| `t` | Process Tree |
| `m` | Metrics |
| `r` | Doctor |
| `s` | Stop selected |
| `f` | Free port |
| `k` | Kill |
| `o` | Open loopback URL |
| `e` | Export snapshot |
| `y` | Copy diagnostic |
| `S` | Toggle system noise |
| `R` | Reload config |
| `?` | Help overlay |
| `[` / `]` / `>` | Sort columns |

See [tui.md](tui.md) and [keybindings.md](keybindings.md) for full
details.

## Using the Web UI

Start the read-only Web UI:

```bash
lazyadmin web --port 7749
```

It binds to loopback only (`127.0.0.1` by default) and refuses
non-loopback addresses. Open `http://127.0.0.1:7749` in a browser.

The Web UI mirrors the TUI views:

- **Overview** — digest with listener counts and triage
- **Listeners** — sortable table with signal and marker chips
- **Workloads** — containers, systemd units, tracked runs
- **Processes** — process tree and details
- **Doctor** — warning groups with actionability metadata
- **Metrics** — adapter event rates and histograms

API endpoints (read-only):

| Endpoint | Description |
|----------|-------------|
| `/api/health` | Health check |
| `/api/snapshot` | Full snapshot JSON |
| `/api/digest` | Overview digest |
| `/api/doctor` | Doctor report |
| `/api/inspector?kind=&id=` | Per-entity inspector |
| `/api/events` | SSE event stream |

Use `--no-open` for headless operation and `--refresh-ms` to control
snapshot polling interval.

## Configuration

lazyadmin reads `~/.config/lazyadmin/config.toml`. Validate it:

```bash
lazyadmin config check --json
```

Example:

```toml
[actions]
require_confirmation = true
open_non_loopback = false
free_multi_owner = "stop_all"

[adapters.sockets]
preferred = "proc"
confirm_dual_stack = true

[ui.theme]
name = "default-dark"

[ui.keybindings.overrides]
quit = "Q"
```

See [keybindings.md](keybindings.md) and [themes.md](themes.md) for UI
configuration.

## Next steps

- Explore [CLI commands](./cli-reference.md)
- Read the [architecture overview](./architecture.md)
- Review [action safety](action-safety.md) before using mutating commands
- Integrate the [agent skill](../skills/lazyadmin-agent/)
