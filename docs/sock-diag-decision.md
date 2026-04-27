# sock_diag discovery decision (PLAN-11)

## Decision

For v0.2, `sock_diag` is an **opt-in spike-safe path** inside `lazyadmin-adapter-procfs`, gated by the crate feature `sock_diag` and the runtime config `adapters.sockets.preferred = "sock_diag" | "both"`.

The default remains `preferred = "proc"`, which keeps v0.1 `/proc/net` behavior unchanged.

## Rationale

- `/proc/net` is portable across the Linux environments lazyadmin targets and does not require elevated capabilities.
- `sock_diag` is useful for parity testing and future richer provenance, but netlink crate/API details and kernel permission differences need more live validation before it becomes default.
- Keeping it in the procfs adapter avoids a second socket adapter with duplicate listener mapping and merge behavior.

## Current v0.2 implementation

- The feature flag exists as `lazyadmin-adapter-procfs/sock_diag`.
- `preferred = "sock_diag"` attempts the opt-in path and falls back to `/proc/net` with a `SOCK_DIAG_DOWNGRADED` warning on any error.
- `preferred = "both"` preserves sock_diag-primary provenance and appends procfs corroborating provenance; parity differences emit `SOCK_DIAG_PARITY_DIFF`.
- The v0.2 spike implementation intentionally reuses procfs fixture enumeration under the sock_diag feature so CI can validate merge/fallback/provenance behavior without requiring live netlink privileges. Native netlink enumeration is deferred until parity has been proven on the target distro matrix.

## Permission posture

Basic listener enumeration may work unprivileged on some kernels, while richer queries can fail with permission or kernel-policy errors. lazyadmin treats any sock_diag failure as degraded, not fatal, and immediately falls back to `/proc/net`.
