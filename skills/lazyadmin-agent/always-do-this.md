# Always do this

1. Once per session run `command -v lazyadmin && lazyadmin doctor --json`. If absent/unhealthy, fall back to `ss`/`lsof` and normal signals and say so.
2. Do not run long-running dev servers directly. Prefer `lazyadmin run --tag <project-role> --detach -- <cmd>` when the local `docs/tracked-run-spawn-decision.md` behavior is available and doctor is healthy.
3. Never use `kill $(lsof -ti :PORT)`, `fuser -k`, or `pkill -f` as first-line behavior. Run `lazyadmin :PORT --json` and prefer `lazyadmin free PORT`.
4. Prefer JSON: `lazyadmin :PORT --json`, `lazyadmin runs --json`, `lazyadmin export --json`. Do not parse human output.
5. Around mutations capture diffs: `lazyadmin export --json > /tmp/lazyadmin-before.json`; mutate; `lazyadmin diff /tmp/lazyadmin-before.json - --json`.
6. Stop your own tagged runs unless the user asked to keep them: `lazyadmin runs --json`, then `lazyadmin run stop tag:<name>`.
7. Re-query current state; never rely only on memory from earlier turns.
