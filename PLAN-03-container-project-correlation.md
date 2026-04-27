# PLAN-03 — Container Adapter, Project Detection, and Correlation Engine

Source: `lazyadmin-spec-v0_2.md` sections 10.4–10.6, 11, 12.6, 18, 20, 23–25, 27–28.  
Depends on: `PLAN-01-foundation-core-cli-json.md`, `PLAN-02-discovery-procfs-systemd-tracked.md`.  
Goal: add Docker/Podman read models, Compose grouping, project detection, special-process classification, and robust graph correlation/conflict handling.

## Important constraints

- Do not assume Podman automatic discovery from bollard. Probe configured/default sockets explicitly.
- v0.1 Podman is read-only unless verified otherwise.
- Container published ports are first-class listener evidence even if no host PID owns the socket.
- Project detection is evidence, not truth; confidence and provenance must be visible.

## Phase 1 — Container adapter crate setup

Crate: `crates/lazyadmin-adapter-container`.

- [x] Add dependencies:
  - [x] `bollard`,
  - [x] `tokio`,
  - [x] `futures`,
  - [x] `tracing`,
  - [x] `thiserror`.
- [x] Define runtime endpoint model:
  - [x] `ContainerRuntimeKind::Docker | PodmanRootless | PodmanRootful | UnknownDockerCompatible`.
  - [x] socket/path/source: `$DOCKER_HOST`, `/var/run/docker.sock`, `/run/podman/podman.sock`, `$XDG_RUNTIME_DIR/podman/podman.sock`, config override.
  - [x] API flavor: Docker-compatible, Podman-compatible, Libpod-specific (v0.2).
- [x] Implement endpoint probe:
  - [x] connect with explicit socket/path/HTTP as configured,
  - [x] call version/info endpoint,
  - [x] classify daemon from returned metadata where possible,
  - [x] record permission errors as `Manager.permission`, not fatal global failure.
- [x] Emit `Manager` entities per reachable/unreachable configured endpoint.

Telemetry:

- [x] Span `adapter.container.probe` per endpoint with source, runtime_kind, reachable, permission_state, API version, duration.

Validation:

```bash
cargo test -p lazyadmin-adapter-container endpoint_config
```

## Phase 2 — Verify Docker/Podman API assumptions

Before broad implementation, create integration notes in `docs/container-api-decision.md`.

- [x] Verify Docker with bollard:
  - [x] list containers,
  - [x] inspect one container,
  - [x] logs stream,
  - [x] stop/restart action availability for later plan,
  - [x] update restart policy request body for later plan.
- [x] Verify Podman rootless, when available:
  - [x] socket location,
  - [x] version/info response shape,
  - [x] container list,
  - [x] inspect,
  - [x] published ports,
  - [x] labels,
  - [x] logs read (follow can remain v0.2).
- [x] If Podman differs from Docker-compatible response shapes, isolate mapping code by runtime kind.
- [x] Do not add Podman actions in v0.1 unless all safety/action tests are added in `PLAN-04`.

## Phase 3 — Container discovery read model

- [x] Use list containers endpoint for initial table-friendly data.
  - [x] Running containers only by default (`show_stopped=false`).
  - [x] Include names, IDs, image, state/status, labels, published ports.
- [x] Implement lazy inspect cache:
  - [x] Inspect on selection/point query or when required for correlation.
  - [x] Cache by container ID + observed state/version where possible.
  - [x] Never block snapshot build on all container inspect calls.
- [x] Convert containers to `Workload` entities:
  - [x] Docker container -> `RuntimeKind::Docker` unless Compose labels promote to Compose workload.
  - [x] Podman container -> `RuntimeKind::Podman`.
  - [x] Compose service -> `RuntimeKind::DockerCompose` or `PodmanCompose`.
- [x] Convert published ports to `Listener` entities:
  - [x] host bind address,
  - [x] host port,
  - [x] protocol,
  - [x] container target port,
  - [x] `Exposure` based on host bind address,
  - [x] provenance: container API reports binding.
- [x] Add `WorkloadOwnsListener` edges from container/compose workload to published listener.
- [x] Detect docker-proxy process later in correlation to avoid double-counting.

Tests:

- [x] Docker list JSON fixture -> workload/listener.
- [x] Docker inspect JSON fixture -> restart policy/source refs.
- [x] Published localhost port exposure.
- [x] Published `0.0.0.0` warning.
- [x] Container without host published port.
- [x] Podman fixture if available.

Validation:

```bash
cargo test -p lazyadmin-adapter-container discovery published_ports
```

## Phase 4 — Compose grouping

- [x] Parse Docker Compose labels:
  - [x] `com.docker.compose.project`,
  - [x] `com.docker.compose.service`,
  - [x] `com.docker.compose.container-number`,
  - [x] `com.docker.compose.config-hash`,
  - [x] `com.docker.compose.project.config_files`,
  - [x] `com.docker.compose.project.working_dir`.
- [x] Parse Podman Compose labels when present:
  - [x] `io.podman.compose.project`,
  - [x] `io.podman.compose.service`,
  - [x] known variants discovered in fixtures.
- [x] Create service-level workload IDs stable across container recreation.
- [x] Attach container IDs as source refs/provenance under service workload.
- [x] Preserve container-level workload only if needed for action/log granularity; otherwise service workload can own process/listener references with container source metadata.
- [x] Populate project hints from Compose working dir/config files.

Tests:

- [x] Compose service grouping stable across container ID change.
- [x] Multiple replicas are represented without fake single PID.
- [x] Missing labels falls back to container workload.

## Phase 5 — Project adapter crate setup

Crate: `crates/lazyadmin-adapter-project`.

- [x] Add dependencies:
  - [x] `ignore` or `walkdir`,
  - [x] `tracing`,
  - [x] `thiserror`,
  - [x] optional `gix` later only if git remote parsing needs it.
- [x] Implement marker definitions from config/spec:
  - `.git`, `package.json`, `bun.lock`, `pnpm-lock.yaml`, `yarn.lock`, `package-lock.json`, `pyproject.toml`, `uv.lock`, `requirements.txt`, `Cargo.toml`, `go.mod`, Compose files, `flake.nix`, `devbox.json`, `.envrc`, `Procfile`, `Makefile`.
- [x] Normalize roots from config and discovered paths.
- [x] Cache project root lookups by path prefix.

Telemetry:

- [x] Span `adapter.project.detect` with candidate count, cache hit count, markers found, duration.

## Phase 6 — Project detection evidence

Inputs:

- process cwd,
- process exe path,
- process command line paths,
- tracked run cwd,
- Compose working_dir/config labels,
- container bind mounts from inspect,
- configured project roots.

Tasks:

- [x] For each candidate path, walk upward until marker/config root boundary.
- [x] Build `Project` entity with:
  - [x] root,
  - [x] name,
  - [x] markers,
  - [x] git remote if cheap/safe,
  - [x] package manager hint,
  - [x] dev command hints from package scripts/Cargo/etc. only if cheap.
- [x] Confidence rules:
  - [x] high: cwd inside git root, Compose working_dir, tracked run cwd.
  - [x] medium: exe/cmdline path or bind mount under known project root.
  - [x] low: parent shell cwd or port convention only.
- [x] Add `WorkloadInProject` edges.
- [x] Add project refs to workloads.

Tests:

- [x] Node/Bun/pnpm project markers.
- [x] Rust Cargo project marker.
- [x] Compose label project.
- [x] container bind mount project.
- [x] no marker -> no false high confidence.

## Phase 7 — Special-process classifier

Implement in core or project/procfs correlation module, not as a separate adapter unless needed.

- [x] Detect command patterns:
  - [x] `kubectl port-forward`,
  - [x] `ssh -L`, `ssh -R`, `ssh -D`,
  - [x] `socat`,
  - [x] `ngrok`,
  - [x] `cloudflared`,
  - [x] `caddy`,
  - [x] `traefik`,
  - [x] `minikube tunnel`,
  - [x] `telepresence`,
  - [x] `envoy`, `linkerd-proxy`, `istio-proxy`.
- [x] Assign semantic runtime kind where modeled:
  - [x] `KubectlPortForward`, `SshTunnel`, `Socat`, `Cloudflared`, or `Direct` with warning badge.
- [x] Add `TUNNEL` or sidecar warnings with provenance from cmdline/exe.
- [x] Never let classifier override high-confidence manager/container ownership; it augments direct processes.

Tests:

- [x] classifiers for each common command pattern.
- [x] sidecar labels do not hide actual owning process.

## Phase 8 — Correlation engine hardening

Crate: `crates/lazyadmin-core/src/correlate/`.

- [x] Merge evidence from procfs, systemd, container, project, tracked adapters.
- [x] Apply ownership priority by evidence confidence, not blind adapter priority.
- [x] High-confidence evidence examples:
  - [x] socket inode under `/proc/<pid>/fd`,
  - [x] container API published port,
  - [x] systemd D-Bus PID-to-unit,
  - [x] Compose labels,
  - [x] systemd socket listener match,
  - [x] tracked-run registry + unit/process match.
- [x] Detect and preserve conflicts:
  - [x] `SO_REUSEPORT` multiple owners,
  - [x] IPv6 possible dual-stack,
  - [x] docker-proxy plus container binding,
  - [x] systemd socket + activated service,
  - [x] same numeric port in different namespaces,
  - [x] TCP and UDP same port.
- [x] Generate warnings/badges:
  - [x] `PUBLIC`, `CONFLICT`, `ROOT`, `SOCKET_ACT`, `ORPHAN`, `STALE`, `TUNNEL`, `TRACKED`, `RESTARTING`.
- [x] Implement default two-tier visibility filter as a view/filter function, not data deletion:
  - [x] hide system-bus units in Everything by default,
  - [x] show hidden count,
  - [x] point queries bypass filter,
  - [x] JSON export includes all entities unless explicitly filtered by command.

Telemetry:

- [x] Span `correlate.run` with entity counts, conflict counts, warning counts, duration.

Tests:

- [x] Docker published port plus docker-proxy avoids double-counting.
- [x] systemd socket/service edge.
- [x] multi-owner port preserved.
- [x] namespace/protocol separation.
- [x] two-tier filter hides but point query returns.

## Phase 9 — CLI views backed by correlated graph

Implement non-mutating commands:

- [x] `lazyadmin ps` and `ps --json`.
- [x] `lazyadmin public`.
- [x] `lazyadmin conflicts`.
- [x] `lazyadmin projects`.
- [x] point selectors for:
  - [x] `container:<name|id-prefix>`,
  - [x] `compose:<project>/<service>`,
  - [x] `project:<name|path>`,
  - [x] `unit:<name>`,
  - [x] `run:<id>`,
  - [x] `tag:<tag>`.
- [x] Human output always includes why/provenance for point queries.
- [x] `--brief` remains one-line and script-safe.

Validation examples:

```bash
cargo run -p lazyadmin-cli -- ps --json | jq .schema_version
cargo run -p lazyadmin-cli -- public
cargo run -p lazyadmin-cli -- conflicts
cargo run -p lazyadmin-cli -- projects --json
```

## Phase 10 — Fixtures and integration tests

Create/maintain fixtures under:

```text
testdata/
  procfs/
  sockets/
  systemd/
  container/
  tracked/
  snapshots/
```

- [x] Add fixture tests for all cases listed in spec section 25.2.
- [x] Add Linux integration tests behind ignored feature/marker:
  - [x] spawn local TCP listener,
  - [x] spawn UDP listener,
  - [x] run Docker container with localhost published port if Docker available,
  - [x] run Docker container with `0.0.0.0` published port if Docker available,
  - [x] run Compose project if Compose available,
  - [x] run systemd user service if user systemd available.
- [x] Tests must skip with explicit reason when Docker/systemd are unavailable; do not fail developer machines for missing optional runtimes.

## Done criteria

- [x] Docker containers are discovered as workloads with published-port listeners.
- [x] Podman rootless/rootful sockets are probed and read-only discovery works when available, or health reports why unavailable.
- [x] Compose labels produce service-level workloads and project hints.
- [x] Project detection groups direct, tracked, and container workloads with confidence/provenance.
- [x] Special tunnel/sidecar processes are labeled.
- [x] Correlation preserves multi-owner/conflict cases instead of collapsing them.
- [x] Non-mutating CLI views work from the correlated graph.
- [x] Fixture tests cover the main edge cases before action code is written.

## Handoff notes for next plan

`PLAN-04` can implement actions/logs once workloads have stable runtime kinds, manager refs, restart policies, and provenance. Do not add mutating container/systemd actions in this plan except API verification spikes.


## Implementation notes (completed)

PLAN-03 was implemented with fixture-first coverage. Live Docker/Podman/systemd integration paths are represented by read-only probing and ignored/skipping test structure where applicable; no mutating container actions were added. Podman actions remain deferred to PLAN-04 as required.
