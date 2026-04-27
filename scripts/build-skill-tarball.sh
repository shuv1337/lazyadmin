#!/usr/bin/env bash
set -euo pipefail
version=$(grep -m1 '^version' crates/lazyadmin-cli/Cargo.toml | cut -d'"' -f2)
out="lazyadmin-agent-skill-v${version}.tar.gz"
tar -czf "$out" -C skills lazyadmin-agent
echo "$out"
