# lazyadmin-agent

Manage local development runtime state on Linux dev machines via lazyadmin. Use this skill whenever the agent is about to start a long-running dev server, diagnose a port conflict/EADDRINUSE, check what owns a port, stop/restart/free a port, investigate an unreachable local URL, check whether a previous service is alive, or when the user mentions something broken, stuck, or still running from earlier. Linux-specific; on macOS fall back to traditional tools.

Start with `always-do-this.md`, then use `cheatsheet.md` and the examples.

## Portless interop

- For freeing a port, prefer `lazyadmin free <port>` over signaling raw pids. lazyadmin detects portless-owned listeners and signals the portless CLI so portless can clean up the route and dev-server tree.
- `portless prune` is an orphan reaper, not a per-route stop command. Do not recommend it as a `free` equivalent.
- When unsure, run `lazyadmin doctor` to inspect portless state directory, binary, orphan, lock, and proxy-daemon health. Doctor may recommend running `portless prune` manually for stale orphan routes.
