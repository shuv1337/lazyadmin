# Container API decision notes

PLAN-03 uses `bollard` against the Docker-compatible API surface for Docker and Podman.

## Endpoint probing

`lazyadmin` probes endpoints explicitly, rather than relying on automatic discovery:

1. `$DOCKER_HOST`
2. `/var/run/docker.sock`
3. `/run/podman/podman.sock`
4. `$XDG_RUNTIME_DIR/podman/podman.sock`

Each endpoint becomes a `Manager` entity. Missing sockets are unavailable managers; permission failures are recorded as `Manager.permission = denied` instead of failing snapshot construction.

## Verified API surface for v0.1 implementation

The implementation is intentionally read-only and relies on:

- version probe (`GET /version` through `bollard::Docker::version`) for reachability/classification.
- container list (`GET /containers/json?all=false` through `list_containers`) for table data, labels, and published ports.
- Docker-compatible list/inspect JSON fixture parsing for stable tests without a live daemon.

## Docker

The Docker-compatible list response contains container IDs, names, image, state/status, labels, and `Ports` entries with `IP`, `PrivatePort`, `PublicPort`, and `Type`. Published ports are represented as first-class `Listener` evidence even if no host PID owns a socket.

Future PLAN-04 action/log work can use the same client for logs, stop/restart, and restart-policy update, but PLAN-03 does not expose mutating operations.

## Podman

Podman is treated as Docker-compatible for v0.1 visibility. Runtime kind is inferred from endpoint path and version/info text when it mentions Podman. Podman remains read-only in this plan; actions are deferred until safety and action tests are added in PLAN-04.

If a Podman daemon returns shape differences not covered by Docker-compatible fixtures, mapping should remain isolated by runtime kind in `lazyadmin-adapter-container`.

## Compose

Docker Compose labels parsed:

- `com.docker.compose.project`
- `com.docker.compose.service`
- `com.docker.compose.container-number`
- `com.docker.compose.config-hash`
- `com.docker.compose.project.config_files`
- `com.docker.compose.project.working_dir`

Podman Compose project/service labels are also accepted using the `io.podman.compose.*` prefix. Service workload IDs use `compose:<project>/<service>` so they survive container recreation.
