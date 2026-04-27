#!/usr/bin/env sh
set -eu
dest="${1:-$HOME/.pi/agent/skills/lazyadmin-agent}"
mkdir -p "$dest"
cp -R . "$dest/"
echo "Installed lazyadmin-agent skill to $dest"
