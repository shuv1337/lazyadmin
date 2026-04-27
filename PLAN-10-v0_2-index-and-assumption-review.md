# PLAN-10 — v0.2 Implementation Index and Assumption Review

Source spec: `lazyadmin-spec-v0_2.md` (sections 6, 10.1, 12.4, 12.6, 18.2, 21.2, 22, 27)
Source v0.1 closeout: `docs/acceptance-v0_1.md`, `PLAN-05` post-v0.1 parking lot
Date: 2026-04-27
Status: Implementation-ready planning set for the v0.2 sprint, scoped to **Discovery upgrades** and **TUI polish**. Out-of-scope items (Podman actions, systemd hardening beyond what discovery needs, packaging targets like Homebrew/Nix) are explicitly deferred to v0.3.

## Sprint scope decisions (confirmed)

- [x] In: discovery upgrades — sock_diag socket source, exact IPv6 dual-stack proof, adapter `watch()` event streams, plus correlation/doctor wiring for the new evidence.
- [x] In: TUI polish — process tree view, metrics panel, configurable keybindings, themes, real Ratatui rendering on top of the v0.1 view-models, live refresh integration with adapter events.
- [x] Out: Podman first-class actions/logs follow — defer to v0.3.
- [x] Out: systemd hardening (sd-journal, scope-mode tracked-run, runtime pause-restart override beyond the v0.1 conservative path) — defer to v0.3.
- [x] Out: packaging (Homebrew tap, Nix flake) — defer to v0.3.
- [x] JSON contract is **additive** in v0.2: `schema_version` stays `lazyadmin.snapshot.v1` / `diff.v1` / `doctor.v1`. New fields are optional; no removals or renames. New event-stream payloads use a new `lazyadmin.discovery_event.v1` schema.

## Plan document map

Implement in order:

1. `PLAN-10-v0_2-index-and-assumption-review.md` (this document)
2. `PLAN-11-v0_2-discovery-upgrades.md`
   - sock_diag adapter path (with `/proc/net` fallback retained)
   - exact IPv6 dual-stack detection via per-socket option proof
   - adapter `watch()` event implementations (procfs poll-debounce, container events, systemd PropertiesChanged) and orchestrator merge
   - new `DiscoveryEvent` JSON contract; doctor signal coverage
3. `PLAN-12-v0_2-tui-polish.md`
   - real Ratatui rendering of all v0.1 view-models (Everything/Ports/Public/Conflicts/Projects/Managers/Orphans/Tracked/Logs/Doctor)
   - Process Tree view (new) and Metrics panel (new)
   - configurable keybindings (TOML overrides, conflict detection)
   - themes (built-ins + user theme files)
   - live refresh wired to `DiscoveryEvent` streams from PLAN-11

A v0.2 release/acceptance pass is **not** a separate plan in this sprint — it is folded into PLAN-12 done criteria plus an updated `docs/acceptance-v0_2.md` snapshot. Heavy CI/release work returns in v0.3.

## Assumption validation summary

### Confirmed for v0.2

- [x] **`netlink-packet-sock-diag` and `netlink-packet-core` exist as published Rust crates.** Maintenance is uneven (last meaningful release ~3 years stale at the time of the spec). The v0.2 plan keeps `/proc/net` as the default and treats `sock_diag` as an opt-in optimization with parity tests against the procfs adapter. If the crate proves unworkable, fall back to a hand-rolled `NETLINK_SOCK_DIAG` client using `nix`/`socket2` rather than dropping the feature.
- [x] **IPv6 dual-stack proof is feasible per-socket.** Linux exposes `IPV6_V6ONLY` via `getsockopt(IPPROTO_IPV6, IPV6_V6ONLY)` on a socket FD. We can read it for FDs we own (i.e., listeners for which we already resolved the owning PID and FD path). `/proc/net/tcp6`/`udp6` alone cannot prove it; the spike confirms FD-level inspection is required.
- [x] **`watch()` already exists on the adapter trait.** PLAN-01 reserved `watch() -> Option<BoxStream<'static, DiscoveryEvent>>`; v0.1 adapters return `None`. v0.2 fills it in for procfs (debounced poll), container (Docker `/events`), and systemd (`PropertiesChanged` signal on relevant interfaces). No trait change required.
- [x] **Ratatui supports the layouts and widgets required for process tree and metrics panel.** `ratatui::widgets::{List, Table, Paragraph, Sparkline, Gauge, Chart}` are stable. `tui-tree-widget` (third-party) provides a tree widget; v0.2 may use it or render the tree manually with `List` rows + ASCII guides. Decision deferred to PLAN-12 Phase 3.

### Assumptions requiring spikes before locking implementation

- [ ] **sock_diag parity vs `/proc/net`.** Before flipping any default, we must show that for every fixture/integration test case used in v0.1, the sock_diag path produces the same `Listener` set with same exposure classification. Spike output: `docs/sock-diag-decision.md` plus a parity test harness.
- [ ] **Per-socket `IPV6_V6ONLY` read path.** We need to validate that we can `getsockopt` on `/proc/<pid>/fd/<n>` opened with `O_PATH` or by re-opening — kernel behavior on FD inheritance through `/proc` is subtle. Spike output: confirmed code path or downgrade `dual_stack_state` to `possible` permanently with explicit doctor note.
- [ ] **Docker events socket reliability.** `bollard::system::events()` returns a stream; we need to confirm reconnection/back-pressure behavior under daemon restarts. Spike output: documented reconnect policy in `docs/discovery-events-decision.md`.
- [ ] **systemd `PropertiesChanged` cost.** Subscribing to all unit property changes on a busy host can be very noisy. Spike: subscribe selectively (only units we already have workloads for) and test under a host with many timers/services.
- [ ] **Configurable-keybinding conflict surface.** Users can override default keys, but multiple bindings to the same key must be rejected with a clear error. Confirm the conflict detection happens at config-load time, not at first key press.
- [ ] **Theme color regression risk on low-color terminals.** Some Linux terminals run in 16-color mode. Themes must declare a fallback palette and degrade rather than rendering invisible text.

## Plan review summary

The v0.1 release shipped the full discovery → correlation → action → TUI-skeleton pipeline. v0.2 trades scope for depth in two dimensions:

- **Discovery freshness**: from poll-only to event-driven where supported, plus higher-fidelity socket evidence.
- **TUI completeness**: from view-models + smoke tests to actually-rendered, themable, customizable panes including two new views.

Both tracks must remain backward-compatible. JSON consumers, agent skills, and existing CLI users should see additive changes only.

## Critical issues carried over from v0.1

### 1. `sock_diag` decision was deferred — must be re-resolved before exposing the option

Reference: `lazyadmin-spec-v0_2.md` §10.1.
Risk: medium — wrong default destabilizes discovery on hosts with restrictive `/proc` semantics.

Resolution in PLAN-11 Phase 2:

- Implement sock_diag as a parallel evidence source, not a replacement.
- `Config.adapters.sockets.preferred` accepts `proc | sock_diag | both`. v0.2 default stays `proc`. Setting to `sock_diag` runs the parity check at startup; on mismatch, doctor warns and the orchestrator falls back to proc with a `SOCK_DIAG_DOWNGRADED` warning.

### 2. Dual-stack badge accuracy

Reference: `lazyadmin-spec-v0_2.md` §12.9, §20; PLAN-00 (open assumption).
Risk: medium — `possible_dual_stack` warnings without resolution become noise.

Resolution in PLAN-11 Phase 3:

- New `Listener.dual_stack_state` enum: `not_applicable | confirmed_dual_stack | confirmed_v6_only | possible | unknown`.
- Default remains `possible` for `[::]` listeners.
- When the FD is reachable and `getsockopt(IPV6_V6ONLY)` succeeds, upgrade to `confirmed_*` and clear the `possible_dual_stack` warning.
- JSON snapshot adds the new field as optional; old consumers ignore it.

### 3. TUI rendering is currently view-model only

Reference: PLAN-05 done criteria; `crates/lazyadmin-tui` source.
Risk: medium — v0.1 ships a TUI binary that exits early or shows a placeholder; the agent skill claims a working TUI in some examples.

Resolution in PLAN-12 Phases 1–4:

- Wire all v0.1 view-models to actual Ratatui widgets behind the existing keymap and dispatcher.
- Treat the new Process Tree and Metrics views as first-class additions; do not regress existing view-models.
- Restore terminal mode on every exit path (already enforced by the v0.1 panic guard; verify in tests).

### 4. Live refresh must not block input

Reference: PLAN-05 Phase 2 performance goals.
Risk: high if PLAN-11 wires events poorly.

Resolution:

- Adapter event streams run on a dedicated `tokio::task`.
- A bounded channel (e.g., `tokio::sync::mpsc::channel(256)`) feeds the snapshot controller. On overflow, drop oldest and emit a `EVENTS_DROPPED` warning.
- Input loop reads from a separate channel; render is triggered by either tick or event arrival.

## Important implementation constraints (carry over and extend)

- [ ] JSON `schema_version` stays at `v1` for snapshot/diff/doctor; v0.2 fields are additive.
- [ ] New stream payloads use `lazyadmin.discovery_event.v1` and ship documented under `docs/schema/`.
- [ ] Every adapter that gains an event source must still support polling fallback when events are unavailable (e.g., Docker daemon down, D-Bus unavailable).
- [ ] Telemetry: add `adapter.watch.start`, `adapter.watch.event`, `adapter.watch.stop`, `tui.render`, `tui.input`, `tui.theme.apply`, `tui.keybind.load` spans/events.
- [ ] All TUI configuration files must be human-editable TOML and validated at load. No silent fallbacks for malformed user themes/keybinds.
- [ ] Process Tree must use `ProcessKey`, not raw PID, for any stable selection or persistence.
- [ ] Metrics panel reads only what is already in core; do not add `/proc` polling outside the procfs adapter.

## Open questions for v0.2 sprint

These are not blockers for plan acceptance; they must resolve before the affected milestone closes:

- [ ] Should `sock_diag` ever auto-promote to default within v0.2, or stay opt-in until v0.3?
- [ ] Should the metrics panel pull rates from a tiny in-process EWMA, or rely on the snapshot diff between refreshes?
- [ ] Should themes ship as TOML files or embedded `serde` constants? (Trade-off: embedded is simpler; TOML is more hackable.)
- [ ] Should the keybinding config also allow per-pane bindings, or remain global only for v0.2?
- [ ] Should the new Process Tree become the default selection target for `t` (currently mapped to "process tree") in PLAN-05's keymap, or remain a sibling pane?

## Done criteria for this v0.2 planning set

- [x] PLAN-11 and PLAN-12 are implementation-ready, ordered, and reference back to this index.
- [x] Each remaining sprint plan includes phases with explicit validation commands and acceptance gates.
- [x] Spike requirements are listed up front so risky work is sequenced before commitments.
- [x] Backward compatibility constraints for JSON, CLI, and agent skill are explicit.
- [x] Out-of-scope themes (Podman actions, systemd hardening, packaging) are documented as v0.3 candidates so future agents do not silently pull them in.
