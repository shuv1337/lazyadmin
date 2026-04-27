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

- [ ] Add dependencies:
  - [ ] `bollard`,
  - [ ] `tokio`,
  - [ ] `futures`,
  - [ ] `tracing`,
  - [ ] `thiserror`.
- [ ] Define runtime endpoint model:
  - [ ] `ContainerRuntimeKind::Docker | PodmanRootless | PodmanRootful | UnknownDockerCompatible`.
  - [ ] socket/path/source: `$DOCKER_HOST`, `/var/run/docker.sock`, `/run/podman/podman.sock`, `$XDG_RUNTIME_DIR/podman/podman.sock`, config override.
  - [ ] API flavor: Docker-compatible, Podman-compatible, Libpod-specific (v0.2).
- [ ] Implement endpoint probe:
  - [ ] connect with explicit socket/path/HTTP as configured,
  - [ ] call version/info endpoint,
  - [ ] classify daemon from returned metadata where possible,
  - [ ] record permission errors as `Manager.permission`, not fatal global failure.
- [ ] Emit `Manager` entities per reachable/unreachable configured endpoint.

Telemetry:

- [ ] Span `adapter.container.probe` per endpoint with source, runtime_kind, reachable, permission_state, API version, duration.

Validation:

```bash
cargo test -p lazyadmin-adapter-container endpoint_config
```

## Phase 2 — Verify Docker/Podman API assumptions

Before broad implementation, create integration notes in `docs/container-api-decision.md`.

- [ ] Verify Docker with bollard:
  - [ ] list containers,
  - [ ] inspect one container,
  - [ ] logs stream,
  - [ ] stop/restart action availability for later plan,
  - [ ] update restart policy request body for later plan.
- [ ] Verify Podman rootless, when available:
  - [ ] socket location,
  - [ ] version/info response shape,
  - [ ] container list,
  - [ ] inspect,
  - [ ] published ports,
  - [ ] labels,
  - [ ] logs read (follow can remain v0.2).
- [ ] If Podman differs from Docker-compatible response shapes, isolate mapping code by runtime kind.
- [ ] Do not add Podman actions in v0.1 unless all safety/action tests are added in `PLAN-04`.

## Phase 3 — Container discovery read model

- [ ] Use list containers endpoint for initial table-friendly data.
  - [ ] Running containers only by default (`show_stopped=false`).
  - [ ] Include names, IDs, image, state/status, labels, published ports.
- [ ] Implement lazy inspect cache:
  - [ ] Inspect on selection/point query or when required for correlation.
  - [ ] Cache by container ID + observed state/version where possible.
  - [ ] Never block snapshot build on all container inspect calls.
- [ ] Convert containers to `Workload` entities:
  - [ ] Docker container -> `RuntimeKind::Docker` unless Compose labels promote to Compose workload.
  - [ ] Podman container -> `RuntimeKind::Podman`.
  - [ ] Compose service -> `RuntimeKind::DockerCompose` or `PodmanCompose`.
- [ ] Convert published ports to `Listener` entities:
  - [ ] host bind address,
  - [ ] host port,
  - [ ] protocol,
  - [ ] container target port,
  - [ ] `Exposure` based on host bind address,
  - [ ] provenance: container API reports binding.
- [ ] Add `WorkloadOwnsListener` edges from container/compose workload to published listener.
- [ ] Detect docker-proxy process later in correlation to avoid double-counting.

Tests:

- [ ] Docker list JSON fixture -> workload/listener.
- [ ] Docker inspect JSON fixture -> restart policy/source refs.
- [ ] Published localhost port exposure.
- [ ] Published `0.0.0.0` warning.
- [ ] Container without host published port.
- [ ] Podman fixture if available.

Validation:

```bash
cargo test -p lazyadmin-adapter-container discovery published_ports
```

## Phase 4 — Compose grouping

- [ ] Parse Docker Compose labels:
  - [ ] `com.docker.compose.project`,
  - [ ] `com.docker.compose.service`,
  - [ ] `com.docker.compose.container-number`,
  - [ ] `com.docker.compose.config-hash`,
  - [ ] `com.docker.compose.project.config_files`,
  - [ ] `com.docker.compose.project.working_dir`.
- [ ] Parse Podman Compose labels when present:
  - [ ] `io.podman.compose.project`,
  - [ ] `io.podman.compose.service`,
  - [ ] known variants discovered in fixtures.
- [ ] Create service-level workload IDs stable across container recreation.
- [ ] Attach container IDs as source refs/provenance under service workload.
- [ ] Preserve container-level workload only if needed for action/log granularity; otherwise service workload can own process/listener references with container source metadata.
- [ ] Populate project hints from Compose working dir/config files.

Tests:

- [ ] Compose service grouping stable across container ID change.
- [ ] Multiple replicas are represented without fake single PID.
- [ ] Missing labels falls back to container workload.

## Phase 5 — Project adapter crate setup

Crate: `crates/lazyadmin-adapter-project`.

- [ ] Add dependencies:
  - [ ] `ignore` or `walkdir`,
  - [ ] `tracing`,
  - [ ] `thiserror`,
  - [ ] optional `gix` later only if git remote parsing needs it.
- [ ] Implement marker definitions from config/spec:
  - `.git`, `package.json`, `bun.lock`, `pnpm-lock.yaml`, `yarn.lock`, `package-lock.json`, `pyproject.toml`, `uv.lock`, `requirements.txt`, `Cargo.toml`, `go.mod`, Compose files, `flake.nix`, `devbox.json`, `.envrc`, `Procfile`, `Makefile`.
- [ ] Normalize roots from config and discovered paths.
- [ ] Cache project root lookups by path prefix.

Telemetry:

- [ ] Span `adapter.project.detect` with candidate count, cache hit count, markers found, duration.

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

- [ ] For each candidate path, walk upward until marker/config root boundary.
- [ ] Build `Project` entity with:
  - [ ] root,
  - [ ] name,
  - [ ] markers,
  - [ ] git remote if cheap/safe,
  - [ ] package manager hint,
  - [ ] dev command hints from package scripts/Cargo/etc. only if cheap.
- [ ] Confidence rules:
  - [ ] high: cwd inside git root, Compose working_dir, tracked run cwd.
  - [ ] medium: exe/cmdline path or bind mount under known project root.
  - [ ] low: parent shell cwd or port convention only.
- [ ] Add `WorkloadInProject` edges.
- [ ] Add project refs to workloads.

Tests:

- [ ] Node/Bun/pnpm project markers.
- [ ] Rust Cargo project marker.
- [ ] Compose label project.
- [ ] container bind mount project.
- [ ] no marker -> no false high confidence.

## Phase 7 — Special-process classifier

Implement in core or project/procfs correlation module, not as a separate adapter unless needed.

- [ ] Detect command patterns:
  - [ ] `kubectl port-forward`,
  - [ ] `ssh -L`, `ssh -R`, `ssh -D`,
  - [ ] `socat`,
  - [ ] `ngrok`,
  - [ ] `cloudflared`,
  - [ ] `caddy`,
  - [ ] `traefik`,
  - [ ] `minikube tunnel`,
  - [ ] `telepresence`,
  - [ ] `envoy`, `linkerd-proxy`, `istio-proxy`.
- [ ] Assign semantic runtime kind where modeled:
  - [ ] `KubectlPortForward`, `SshTunnel`, `Socat`, `Cloudflared`, or `Direct` with warning badge.
- [ ] Add `TUNNEL` or sidecar warnings with provenance from cmdline/exe.
- [ ] Never let classifier override high-confidence manager/container ownership; it augments direct processes.

Tests:

- [ ] classifiers for each common command pattern.
- [ ] sidecar labels do not hide actual owning process.

## Phase 8 — Correlation engine hardening

Crate: `crates/lazyadmin-core/src/correlate/`.

- [ ] Merge evidence from procfs, systemd, container, project, tracked adapters.
- [ ] Apply ownership priority by evidence confidence, not blind adapter priority.
- [ ] High-confidence evidence examples:
  - [ ] socket inode under `/proc/<pid>/fd`,
  - [ ] container API published port,
  - [ ] systemd D-Bus PID-to-unit,
  - [ ] Compose labels,
  - [ ] systemd socket listener match,
  - [ ] tracked-run registry + unit/process match.
- [ ] Detect and preserve conflicts:
  - [ ] `SO_REUSEPORT` multiple owners,
  - [ ] IPv6 possible dual-stack,
  - [ ] docker-proxy plus container binding,
  - [ ] systemd socket + activated service,
  - [ ] same numeric port in different namespaces,
  - [ ] TCP and UDP same port.
- [ ] Generate warnings/badges:
  - [ ] `PUBLIC`, `CONFLICT`, `ROOT`, `SOCKET_ACT`, `ORPHAN`, `STALE`, `TUNNEL`, `TRACKED`, `RESTARTING`.
- [ ] Implement default two-tier visibility filter as a view/filter function, not data deletion:
  - [ ] hide system-bus units in Everything by default,
  - [ ] show hidden count,
  - [ ] point queries bypass filter,
  - [ ] JSON export includes all entities unless explicitly filtered by command.

Telemetry:

- [ ] Span `correlate.run` with entity counts, conflict counts, warning counts, duration.

Tests:

- [ ] Docker published port plus docker-proxy avoids double-counting.
- [ ] systemd socket/service edge.
- [ ] multi-owner port preserved.
- [ ] namespace/protocol separation.
- [ ] two-tier filter hides but point query returns.

## Phase 9 — CLI views backed by correlated graph

Implement non-mutating commands:

- [ ] `lazyadmin ps` and `ps --json`.
- [ ] `lazyadmin public`.
- [ ] `lazyadmin conflicts`.
- [ ] `lazyadmin projects`.
- [ ] point selectors for:
  - [ ] `container:<name|id-prefix>`,
  - [ ] `compose:<project>/<service>`,
  - [ ] `project:<name|path>`,
  - [ ] `unit:<name>`,
  - [ ] `run:<id>`,
  - [ ] `tag:<tag>`.
- [ ] Human output always includes why/provenance for point queries.
- [ ] `--brief` remains one-line and script-safe.

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

- [ ] Add fixture tests for all cases listed in spec section 25.2.
- [ ] Add Linux integration tests behind ignored feature/marker:
  - [ ] spawn local TCP listener,
  - [ ] spawn UDP listener,
  - [ ] run Docker container with localhost published port if Docker available,
  - [ ] run Docker container with `0.0.0.0` published port if Docker available,
  - [ ] run Compose project if Compose available,
  - [ ] run systemd user service if user systemd available.
- [ ] Tests must skip with explicit reason when Docker/systemd are unavailable; do not fail developer machines for missing optional runtimes.

## Done criteria

- [ ] Docker containers are discovered as workloads with published-port listeners.
- [ ] Podman rootless/rootful sockets are probed and read-only discovery works when available, or health reports why unavailable.
- [ ] Compose labels produce service-level workloads and project hints.
- [ ] Project detection groups direct, tracked, and container workloads with confidence/provenance.
- [ ] Special tunnel/sidecar processes are labeled.
- [ ] Correlation preserves multi-owner/conflict cases instead of collapsing them.
- [ ] Non-mutating CLI views work from the correlated graph.
- [ ] Fixture tests cover the main edge cases before action code is written.

## Handoff notes for next plan

`PLAN-04` can implement actions/logs once workloads have stable runtime kinds, manager refs, restart policies, and provenance. Do not add mutating container/systemd actions in this plan except API verification spikes.
