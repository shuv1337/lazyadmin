# Troubleshooting

lazyadmin is Linux-first. Run `lazyadmin doctor --json` for structured permission and adapter status. Common gaps: no Docker/Podman socket, systemd user bus unavailable in containers/CI, permission-denied `/proc/<pid>/fd` walks, and terminals narrower than 60 columns refusing the TUI. Podman is read-only in v0.1.
