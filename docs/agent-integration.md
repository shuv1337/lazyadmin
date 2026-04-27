# Agent integration

The `skills/lazyadmin-agent/` pack teaches coding agents to check `lazyadmin doctor --json`, wrap long-running commands with `lazyadmin run --tag ... --detach -- <cmd>`, prefer JSON, avoid unsafe kill patterns, capture diffs around mutations, and stop their own tagged runs. Build the release tarball with `scripts/build-skill-tarball.sh`.
