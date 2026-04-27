# lazyadmin — Product & Engineering Specification

Version: 0.2
Status: Draft for implementation
Primary platform: Linux
Primary stack: Rust + Ratatui from day one
Last updated: April 27, 2026

Changelog from v0.1:
- MVP scope trimmed: Podman ships as read-only discovery, actions move to v0.2.
- Container adapter unified: bollard speaks both Docker and Podman, so the spec collapses two adapters into one.
- Socket discovery primary path inverted: `/proc/net` is primary for v0.1, sock_diag is a v0.2 optimization once perf data justifies it.
- New runtime: `lazyadmin run` wrapper with systemd-run primary path. Tracked processes are first-class workloads with capturable logs.
- Agent integration: ships with a `lazyadmin-agent` skill so coding agents use the wrapper instead of bare `npm run dev`, and query JSON instead of guessing.
- CLI surface: `explain` command merged into bare port query. `--brief` flag added for one-line scripting output. `lazyadmin diff` added.
- Free-port semantics: multi-owner ports now stop all owners atomically with best-effort reporting per owner.
- New action: `pause-restart` to temporarily disable Docker/systemd restart policies so a stop sticks.
- Visibility model: two-tier default that hides system bus units; point queries always bypass the filter.
- Verify model: factual diff reporting, no inference about whether a manager-driven re-bind counts as success.
- Adapter trait: `watch()` method defined now (returning `Option<BoxStream<DiscoveryEvent>>`), even though all v0.1 adapters return `None`. This wires the orchestrator for v0.2 eventing without restructuring.

---

## 1. One-line description

`lazyadmin` is a terminal-native local runtime control plane for developers. It shows what is running on the machine, what is listening on which port, which runtime owns it, which project it belongs to, what logs explain it, and what manager-aware action safely stops, restarts, opens, or frees it.

It is not a better `ps`, a prettier `lsof`, a Docker clone, or a systemd clone. It is the missing correlation layer between all of them, plus a process wrapper that lets developers and AI agents start work in a way that's traceable from the start.

---

## 2. Core product thesis

Developers do not really ask, "what process is using port 3000?"

They ask:

```text
Why is my local dev server broken?
What is running?
Where did it come from?
Is it Docker, Compose, systemd, Podman, launchd, or some orphaned shell child?
Can I free this port without killing the wrong thing?
Where are the logs?
Why is this bound to 0.0.0.0 like a menace?
```

`lazyadmin` answers those questions by building a normalized runtime graph from host sockets, processes, service managers, container runtimes, logs, and project markers.

The killer feature is not killing a PID. That is table stakes. The killer feature is correct ownership with provenance:

```text
tcp://127.0.0.1:3000
  owner: web
  runtime: lazyadmin-tracked (or direct, if not wrapped)
  command: bun run dev
  cwd: ~/src/acme/web
  parent: tmux -> zsh
  action: stop tracked run, not random SIGKILL

why:
  /proc/net/tcp listener inode 123456
  /proc/42420/fd/17 points to socket:[123456]
  process cwd is inside ~/src/acme/web
  package.json contains dev script
```

The second-killer feature is `lazyadmin run`: developers and agents wrap their dev servers from the start, so the runtime graph is correct on the first scan instead of reconstructed from clues after the fact.

---

## 3. Inspiration projects

`lazyadmin` intentionally combines product instincts from three projects, plus an explicit step beyond what any of them attempt.

| Project | What to steal shamelessly | What not to copy blindly |
|---|---|---|
| `isd` | Keyboard-first systemd UX, fuzzy search, previews, command palette, sudo-awareness, custom keybindings, user/system unit switching | Do not become systemd-only. systemd is one adapter in a larger ownership graph. |
| `lazydocker` | All common runtime actions one keypress away; logs, status, resources, and service operations in one terminal window | Do not become Docker-only. Docker is often just one reason a port is busy. |
| `port` | Simple local painkiller: list open ports, search, and free them | Do not model ownership as "one port equals one PID." That falls apart with Docker, systemd sockets, namespaces, and `SO_REUSEPORT`. |

The step beyond: a process wrapper (`lazyadmin run`) that puts new work into the graph correctly from the start, and a JSON contract that lets coding agents participate as first-class users.

---

## 4. Design principles

### 4.1 Graph first, UI second

The UI is a projection of a normalized graph. The core engine works without the TUI and exports clean JSON. Every UI affordance has a CLI/JSON equivalent.

### 4.2 Ownership over raw facts

A listener row should not merely say:

```text
3000 node 42420
```

It should say:

```text
3000 web lazyadmin-tracked ~/src/acme/web bun run dev (tag: acme-web)
```

The central problem is correlation, not listing.

### 4.3 Manager-aware actions

Never kill the leaf process when a safer runtime-aware operation exists.

Preferred order:

```text
Docker Compose service stop
Docker container stop
Podman container stop (read-only in v0.1; actions in v0.2)
systemd StopUnit
systemd socket StopUnit
lazyadmin run stop                       # tracked-run wrapper
launchctl bootout / kickstart equivalent (post-MVP)
SIGTERM process group
SIGTERM PID
SIGKILL only after explicit escalation
```

### 4.4 Provenance everywhere

Every ownership claim must include why the system believes it.

Examples:

```text
socket inode -> /proc fd -> PID
PID -> systemd unit via D-Bus GetUnitByPIDFD
PID -> systemd unit via /proc/<pid>/cgroup parse (bulk fast path)
container -> Docker API inspect
compose service -> canonical Compose labels
project -> cwd under git root
tracked run -> lazyadmin run registry entry
```

### 4.5 No privileged daemon by default

Run unprivileged. Show partial information when permissions block discovery. Escalate only for a specific action or targeted rediscovery.

### 4.6 Linux-first, portable by adapters

The MVP is Linux-first. macOS support is a later adapter. Windows native support is out of scope for v1.

### 4.7 Boring and correct beats fancy and wrong

No eBPF dependency in MVP. No AI magic. No "auto-fix everything." No faceplanting into `sudo kill -9` because a table row looked sad.

### 4.8 Honesty over inference

When `lazyadmin` cannot prove something, it says so. When a managed restart re-binds a port immediately after a stop, the verify step reports the factual ownership change without judging whether that's success or failure. The user decides.

---

## 5. Primary users

### 5.1 Local full-stack developer

Runs frontend dev servers, API servers, Redis/Postgres containers, tunnels, local reverse proxies, and half-forgotten experiments.

Needs:

```text
What owns :3000, :5173, :5432, :6379?
Which project is it from?
Can I stop it safely?
Can I see logs immediately?
```

### 5.2 Platform / infra engineer

Has systemd user services, Docker/Podman containers, local Kubernetes port-forwards, reverse proxies, and test services.

Needs:

```text
Is this service systemd, Docker, Podman, or raw?
Why is it reachable on the LAN?
Which manager should I operate through?
```

### 5.3 Power user with too much local state

Uses tmux, direnv, Nix, devbox, asdf, language-specific tools, and many repos.

Needs:

```text
Group everything by project.
Find orphaned processes.
Copy useful diagnostics.
Do not make me remember 17 commands.
```

### 5.4 AI coding agent

Spawns dev servers, build watchers, and test harnesses on behalf of a human user. Needs to find what it started three turns later, stop it cleanly, capture logs, and report state without guessing. Today, agents fall back to `lsof -ti :3000 | xargs kill -9`, which is exactly the kind of thing this tool exists to prevent.

Needs:

```text
A wrapper that tags spawned processes for later lookup.
Stable JSON output for queries.
Non-destructive defaults.
A skill or doc that teaches it the contract.
```

---

## 6. Non-goals

| Non-goal | Reason |
|---|---|
| Production monitoring | This is local/dev runtime control, not Prometheus. |
| Full Docker replacement | `lazyadmin` correlates Docker with everything else. |
| Full systemd replacement | `lazyadmin` uses systemd as a runtime adapter. |
| Kubernetes dashboard | Port-forward detection is in scope; full Kubernetes management is not MVP. |
| Security scanner | Public-bind and privilege warnings are in scope; vulnerability scanning is not. |
| Background root agent | Too much risk and installation friction for v1. |
| Magic auto-remediation | Dangerous actions require explicit confirmation. |
| Web dashboard | TUI and CLI first. |
| Process supervisor replacement | `lazyadmin run` is a wrapper for tracking and lifecycle, not a supervisord/pm2 replacement with restart policies, dependency graphs, and health checks. |

---

## 7. Recommended implementation stack

### 7.1 Decision

Use Rust + Ratatui from day one.

### 7.2 Rationale

The hard part is accurate host introspection and correlation:

```text
socket -> inode -> PID -> namespace -> cgroup -> manager -> workload -> project
```

That fits Rust better than a fast Python prototype. The project wants a durable core, a single binary, fast scanning, good error handling, low memory overhead, and confidence around low-level Linux interfaces.

### 7.3 UI stack

```text
ratatui            main TUI crate, continuation of tui-rs
crossterm          terminal backend
tokio              async runtime
clap               CLI parsing
serde              serialization
serde_json         JSON output
toml               config
tracing            structured logging
color-eyre         error reporting
```

### 7.4 Runtime adapter stack

```text
zbus               systemd D-Bus (system + user buses)
nix                Unix process/signal helpers, prctl, setns
procfs             /proc parsing for processes and /proc/net/* for sockets
bollard            Docker AND Podman client (Podman supported as first-class)
hyper / hyperlocal HTTP over Unix sockets if a custom client becomes necessary later
notify             config/project file watching
ignore / walkdir   project root scanning
which              fallback command availability
nix (cgroup helpers, optional) cgroup v2 manipulation for the manual run fallback
```

Notable choices:

- **bollard for both Docker and Podman**: bollard ships first-class Podman support with automatic socket discovery for rootless and rootful sockets. This collapses what was previously two adapters into one.
- **`/proc/net` parsing as primary in v0.1**: the `netlink-packet-sock-diag` crate is functional but has not seen meaningful maintenance in nearly three years. Rather than depend on a stale crate or roll a netlink client by hand, v0.1 parses `/proc/net/{tcp,tcp6,udp,udp6,unix}` directly via the `procfs` crate. sock_diag is a v0.2 optimization once we have real performance data from busy machines.

### 7.5 Shelling out policy

Shell out only as a fallback or compatibility adapter.

Preferred APIs:

```text
systemd: D-Bus
systemd-run: D-Bus method calls or systemd-run binary (binary acceptable for v0.1)
Docker / Podman: Engine API via bollard
Sockets: /proc/net parsing (sock_diag in v0.2)
Processes: /proc
Logs: journal API or journalctl fallback; Docker/Podman logs via bollard
```

Allowed fallbacks (must be marked in provenance when used):

```text
ss
lsof
journalctl
systemctl
launchctl
podman
docker
```

---

## 8. High-level architecture

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                          lazyadmin CLI/TUI                              │
├─────────────────────────────────────────────────────────────────────────┤
│ Views: Everything | Ports | Projects | Managers | Logs | Doctor | Runs  │
│ Commands: port/free/ps/public/conflicts/logs/doctor/export/run/diff     │
├─────────────────────────────────────────────────────────────────────────┤
│                       Core runtime graph engine                          │
│                                                                          │
│ Snapshot builder -> Correlator -> Action planner -> Presentation         │
├──────────────────────┬──────────────────────┬───────────────────────────┤
│ Discovery adapters   │ Log adapters         │ Action adapters           │
│ - sockets (/proc)    │ - journal            │ - systemd D-Bus           │
│ - proc               │ - container logs     │ - container (Docker/Podman)│
│ - systemd            │   (Docker/Podman)    │ - lazyadmin-tracked       │
│ - container          │ - tracked run logs   │ - Unix signals            │
│ - project roots      │   (journal or file)  │ - launchctl (post-MVP)    │
│ - lazyadmin-tracked  │                      │                           │
└──────────────────────┴──────────────────────┴───────────────────────────┘
```

`lazyadmin-tracked` appears in all three columns: it discovers its own runs via a registry under `$XDG_RUNTIME_DIR/lazyadmin/runs/`, it provides logs (via journal when systemd-run is the path, or a captured log file in the manual fallback), and it provides actions (start, stop, restart, forget).

---

## 9. Core normalized graph

### 9.1 Entity types

```rust
pub enum RuntimeKind {
    Direct,
    LazyadminTracked,        // started via `lazyadmin run`
    SystemdSystem,
    SystemdUser,
    SystemdSocket,
    Docker,
    DockerCompose,
    Podman,
    PodmanCompose,
    PodmanPod,
    Launchd,                 // post-MVP
    Supervisor,              // future plugin adapter
    KubectlPortForward,
    SshTunnel,
    Socat,
    Cloudflared,
    Unknown,
}

pub enum Confidence {
    High,
    Medium,
    Low,
}

pub enum Exposure {
    Loopback,
    LanOrPublic,
    Public,
    ContainerOnly,
    UnixLocal,
    Unknown,
}
```

### 9.2 Confidence aggregation rule

Both `Listener` and `Provenance` carry a confidence. The listener's confidence is the **maximum** confidence across its provenance entries (highest evidence wins). The reasoning: one piece of high-confidence evidence (sock_diag inode match, Docker API published port) is enough to claim ownership; weaker corroborating evidence does not weaken it. Conflicting evidence is handled separately as a conflict (section 11.3), not as a confidence reduction.

### 9.3 Listener

```rust
pub struct Listener {
    pub id: ListenerId,
    pub protocol: Protocol,
    pub family: AddressFamily,
    pub bind_addr: Option<String>,
    pub port: Option<u16>,
    pub path: Option<PathBuf>,
    pub state: ListenerState,
    pub netns: NamespaceId,
    pub socket_inode: Option<u64>,
    pub exposure: Exposure,
    pub owners: Vec<OwnerRef>,
    pub confidence: Confidence,
    pub provenance: Vec<Provenance>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}
```

### 9.4 Process

```rust
pub struct Process {
    pub key: ProcessKey,
    pub pid: i32,
    pub start_time_ticks: u64,
    pub boot_id: String,
    pub user: Option<String>,
    pub exe: Option<PathBuf>,
    pub cmdline: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub ppid: Option<i32>,
    pub pgid: Option<i32>,
    pub sid: Option<i32>,
    pub cgroup: Option<String>,
    pub netns: Option<NamespaceId>,
    pub container_id: Option<String>,
    pub systemd_unit: Option<String>,
    pub lazyadmin_run_id: Option<RunId>,
    pub environment: RedactedEnvironmentSummary,
    pub provenance: Vec<Provenance>,
}

pub struct ProcessKey {
    pub pid: i32,
    pub boot_id: String,
    pub start_time_ticks: u64,
}
```

PID alone is not stable. PIDs recycle. Use `(pid, boot_id, start_time)`.

### 9.5 Workload

```rust
pub struct Workload {
    pub id: WorkloadId,
    pub display_name: String,
    pub runtime: RuntimeKind,
    pub state: WorkloadState,
    pub pids: Vec<ProcessRef>,
    pub listeners: Vec<ListenerRef>,
    pub project: Option<ProjectRef>,
    pub manager: Option<ManagerRef>,
    pub source: Option<SourceRef>,
    pub actions: Vec<Action>,
    pub health: Option<HealthSummary>,
    pub metrics: Option<MetricsSummary>,
    pub restart_policy: Option<RestartPolicy>,
    pub lazyadmin_run_id: Option<RunId>,
    pub provenance: Vec<Provenance>,
}

pub struct RestartPolicy {
    pub source: RestartPolicySource,   // SystemdRestart, DockerRestart, None
    pub policy: String,                // "always" | "on-failure" | "no" | "unless-stopped"
    pub raw: String,                   // raw value from manager
}
```

`restart_policy` is populated when the manager makes it observable: Docker exposes `HostConfig.RestartPolicy`, systemd exposes `Restart=` on the unit. This is what powers the verify step's factual reporting and the `pause-restart` action.

### 9.6 Manager

```rust
pub struct Manager {
    pub id: ManagerId,
    pub kind: RuntimeKind,
    pub name: String,
    pub scope: ManagerScope,
    pub socket: Option<PathBuf>,
    pub available: bool,
    pub permission: PermissionState,
    pub version: Option<String>,
    pub provenance: Vec<Provenance>,
}
```

### 9.7 Project

```rust
pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    pub markers: Vec<ProjectMarker>,
    pub git_remote: Option<String>,
    pub package_manager: Option<String>,
    pub dev_commands: Vec<DevCommandHint>,
    pub provenance: Vec<Provenance>,
}
```

### 9.8 Edge taxonomy

Edges connect entities in the graph. Defined edge kinds:

```rust
pub enum EdgeKind {
    ProcessOwnsListener,       // process holds the socket FD
    WorkloadContainsProcess,   // workload's process set
    WorkloadOwnsListener,      // derived from contained process(es)
    ManagerOwnsWorkload,       // systemd unit -> service workload, etc.
    WorkloadInProject,         // project containment
    WorkloadActivatedBy,       // socket-activated service edge
    TrackedRunSpawned,         // lazyadmin run -> child workload
}

pub struct Edge {
    pub kind: EdgeKind,
    pub from: EntityRef,
    pub to: EntityRef,
    pub provenance: Vec<Provenance>,
}
```

Edges are emitted alongside entities in `DiscoveryOutput` and surface in JSON export.

### 9.9 Provenance

```rust
pub struct Provenance {
    pub adapter: String,
    pub claim: String,
    pub evidence: String,
    pub confidence: Confidence,
    pub timestamp: DateTime<Utc>,
}
```

Example:

```json
{
  "adapter": "procfs",
  "claim": "pid 42420 owns socket inode 123456",
  "evidence": "/proc/42420/fd/17 -> socket:[123456]",
  "confidence": "high"
}
```

---

## 10. Discovery adapters

### 10.1 Socket adapter

#### Objective

Discover listening network sockets and local Unix sockets.

#### v0.1 primary path

Parse `/proc/net/{tcp,tcp6,udp,udp6,unix}` via the `procfs` crate. This is the path we ship.

Reasoning: the `netlink-packet-sock-diag` crate is at version 0.4.2 and has not been updated in nearly three years. The wider rust-netlink ecosystem is healthy for route/audit/wireguard, but sock_diag specifically is stale. Rather than depend on unmaintained code or roll a custom netlink client for v0.1, we parse `/proc/net` directly. The format is documented, stable for two decades, and gives us the exact listener inode set we need.

For typical dev workstations (50 to 200 listeners), the perf cost over sock_diag is sub-millisecond. The trade is real only on servers with thousands of sockets, which is not our target.

#### v0.1 fallback order

```text
1. /proc/net/{tcp,tcp6,udp,udp6,unix}        primary
2. ss -H -ltnup / ss -H -lunp                if /proc readable but parse fails
3. lsof                                      last resort
```

#### v0.2 path

Add sock_diag as an opt-in optimized path. Either depend on `netlink-packet-sock-diag` if it has seen maintenance by then, or write a minimal hand-rolled netlink client (the `INET_DIAG_BY_FAMILY` request is small; 300 to 400 lines of careful Rust). Selection is config-driven with feature detection at startup.

#### Listener key

Never treat a port as globally unique.

```rust
pub struct ListenerKey {
    pub netns: NamespaceId,
    pub protocol: Protocol,
    pub family: AddressFamily,
    pub bind_addr: String,
    pub port: Option<u16>,
    pub socket_inode: Option<u64>,
}
```

These are different listeners:

```text
tcp/127.0.0.1:3000 in host netns
tcp/0.0.0.0:3000 in host netns
udp/0.0.0.0:3000 in host netns
tcp/127.0.0.1:3000 in container netns
unix:///tmp/app.sock
```

#### Cross-namespace honesty

Unprivileged enumeration of sockets inside a non-host network namespace requires `setns()` which requires `CAP_SYS_ADMIN`. `lazyadmin` running as a regular user cannot scan inside container netns directly. For containers, listeners are derived from the container API published port mappings, not from sock_diag inside the container. The doctor command surfaces this limitation explicitly. If a user needs full container-internal visibility, they run `lazyadmin` with elevated permissions for that scan only.

#### Exposure classification

```text
127.0.0.1, ::1                  -> loopback
0.0.0.0, ::                     -> lan_or_public
specific private LAN IP         -> lan_or_public
public routable IP              -> public
container bridge only           -> container_only
unix socket path                -> unix_local
unknown namespace               -> unknown
```

Use careful wording. `0.0.0.0` means not localhost-only; it does not prove internet exposure. The UI says "reachable beyond localhost depending on firewall/routing," not "the entire internet is in your toaster."

### 10.2 Process adapter

#### Objective

Map sockets to owning processes and enrich those processes.

#### Data sources

```text
/proc/<pid>/fd/*       socket:[inode]
/proc/<pid>/exe        executable
/proc/<pid>/cmdline    command line
/proc/<pid>/cwd        working directory
/proc/<pid>/status     uid/gid, ppid, state
/proc/<pid>/stat       start time, process group, session
/proc/<pid>/cgroup     cgroup path (used for systemd unit inference, fast path)
/proc/<pid>/ns/net     network namespace identifier
/proc/<pid>/environ    opt-in, redacted
/proc/sys/kernel/random/boot_id
```

#### Scan strategy and perf

The naive walk (every process, every FD, readlink each) is O(processes × fds) and can run into 10k+ readlinks on a busy box. The order we use:

```text
1. Read listener inode set from /proc/net/* once.
2. Walk /proc, collect (pid, start_time) for cache validation.
3. For each process whose start_time is unchanged since last scan, reuse cached metadata.
4. For processes that need re-enrichment, readlink fds only until we find sockets in the listener inode set, then stop.
5. Cache by ProcessKey across scans.
```

This hits the section 24 perf budgets on machines with hundreds of processes.

#### Permission behavior

When a process cannot be read:

```text
show PID if known
show owner uid if known
mark details as permission denied
offer targeted sudo rediscovery
```

Do not silently hide rows. Hidden rows are exactly how people waste an hour.

### 10.3 systemd adapter

#### Objective

Correlate processes and sockets to systemd system/user units, support logs, and perform manager-aware actions.

#### Buses

Connect to both:

```text
system bus -> system services
user bus   -> user services
```

#### Correlation strategy: bulk fast path plus targeted verification

For bulk discovery: parse `/proc/<pid>/cgroup` and infer the unit from the cgroup path (`/system.slice/foo.service`, `/user.slice/[email protected]/app.slice/foo.service`). This is what `systemd-cgls` does. Race-free with start_time validation, no per-process D-Bus call.

For targeted lookups (user clicks on an entity): use D-Bus `GetUnitByPIDFD` for race-free verification, falling back to `GetUnitByPID` if the kernel/systemd version is too old.

#### D-Bus methods of interest

```text
GetUnitByPIDFD              preferred for race-free targeted lookup
GetUnitByPID                fallback
GetUnitByControlGroup       useful for cgroup-derived lookups
ListUnits
ListUnitFiles
StartUnit
StopUnit
RestartUnit
KillUnit
MaskUnitFiles               used by `pause-restart`
UnmaskUnitFiles
```

#### Socket units

Socket units are first-class workload owners.

Systemd socket units can define:

```text
ListenStream=          TCP / stream sockets
ListenDatagram=        UDP / datagram sockets
ListenSequentialPacket= AF_UNIX seqpacket
Service=               activated service
Accept=                instance per connection
ReusePort=             multiple bind support
BindIPv6Only=          IPv4/IPv6 dual-stack behavior
SocketUser=            socket-unit user (may differ from service user)
SocketGroup=           socket-unit group
```

Required behavior:

```text
:8080 owned by foo.socket
foo.service inactive
incoming traffic activates foo.service
```

The model represents this as two workloads (socket and service) connected by a `WorkloadActivatedBy` edge.

#### Restart policy detection

The systemd adapter populates `Workload.restart_policy` for each service workload by reading the `Restart=` directive from the unit's properties. Values: `no`, `on-success`, `on-failure`, `on-abnormal`, `on-watchdog`, `on-abort`, `always`. This is what powers the verify step and the `pause-restart` action.

#### Logs

Use the journal for systemd logs.

Preferred:

```text
sd-journal bindings or journal API where stable
```

Fallback:

```text
journalctl --unit <unit> --follow --output=json
journalctl _PID=<pid> --follow --output=json
```

### 10.4 Container adapter (Docker + Podman)

A single adapter speaks both Docker and Podman via the bollard crate. bollard supports Podman as a first-class runtime with automatic socket discovery for rootless and rootful sockets, so two clients are not needed.

#### Endpoints / operations (Docker and Podman, common subset)

```text
GET /version
GET /containers/json?all=false
GET /containers/{id}/json
GET /containers/{id}/logs?stdout=true&stderr=true&follow=true
GET /containers/{id}/stats?stream=true
GET /events                 (v0.2 eventing)
POST /containers/{id}/stop
POST /containers/{id}/restart
POST /containers/{id}/kill
POST /containers/{id}/update    (used for `pause-restart`: set RestartPolicy=no)
GET /networks
GET /volumes
```

For Podman-specific endpoints (Libpod pods, etc.), bollard exposes them under its Podman API surface. Pods are surfaced as a `PodmanPod` workload kind grouping member containers, but pod actions are post-MVP.

#### Socket discovery

Detect in this order:

```text
$DOCKER_HOST                                 if set
/var/run/docker.sock                         Docker default
/run/podman/podman.sock                      Podman rootful
$XDG_RUNTIME_DIR/podman/podman.sock          Podman rootless
                                             (typically /run/user/<uid>/podman/podman.sock)
```

Both are probed at startup. Available sockets become `Manager` entries with the appropriate kind.

#### Performance: do not block UI on per-container inspect

`containers/json` is fast and returns the published port info we need for the table view. Per-container `inspect` calls are only made when the user expands a row or the workload becomes the current selection. On a box with 30+ containers this prevents the initial snapshot from stalling on cumulative round-trip time.

#### Published ports

Docker and Podman published ports are not host processes listening on the published port. They map published container ports to host IPs using firewall/NAT/PAT rules. The container API port bindings are first-class listener evidence, even when no host PID owns the host-facing socket.

Represent both sides:

```text
host binding:       0.0.0.0:5432/tcp
container target:   postgres:5432/tcp
runtime owner:      Docker container or Compose service
```

#### Compose detection

Use canonical Compose labels:

```text
com.docker.compose.project
com.docker.compose.service
```

Optional labels collected when present:

```text
com.docker.compose.container-number
com.docker.compose.config-hash
com.docker.compose.project.config_files
com.docker.compose.project.working_dir
```

Podman Compose uses similar labels with `io.podman.compose` prefixes; collect both.

#### Restart policy detection

Populate `Workload.restart_policy` from `HostConfig.RestartPolicy.Name` (Docker) or the equivalent Podman field. Values: `no`, `always`, `unless-stopped`, `on-failure`. The `pause-restart` action uses `containers/{id}/update` with `RestartPolicy.Name=no` to disable automatic restart before a stop.

#### Public bind warnings

Published ports bound to non-loopback addresses are flagged:

```text
0.0.0.0:5432 -> container:5432/tcp
warning: published beyond localhost; check firewall/routing
```

#### Container socket security

Access to the Docker socket is privileged. The Docker daemon documents that controlling the daemon is root-equivalent in common configurations, and the `docker` group grants root-level privileges. Doctor warns when the user has direct Docker socket access.

`lazyadmin` must not:

```text
chmod docker.sock
recommend blindly adding users to docker group
run arbitrary container operations without confirmation
hide socket permission risk
```

#### Podman in v0.1

Read-only: discovery, listing, inspection, logs (read), labels, published ports. No actions, no log follow. Doctor reports the Podman socket as healthy and notes "actions coming in v0.2." This trims roughly a month of action/log adapter work without losing the visibility benefits.

### 10.5 Project adapter

#### Objective

Group workloads by development project.

#### Inputs

```text
process cwd
process exe path
process command line
container labels
compose working_dir label
mounts / bind mounts
git root
known project roots from config
```

#### Markers

```text
.git
package.json
bun.lock
pnpm-lock.yaml
yarn.lock
package-lock.json
pyproject.toml
uv.lock
requirements.txt
Cargo.toml
go.mod
compose.yaml
compose.yml
docker-compose.yaml
docker-compose.yml
flake.nix
devbox.json
.envrc
Procfile
Makefile
```

#### Confidence rules

High confidence:

```text
cwd is inside git root
container has Compose working_dir/config_files labels
process command references marker in same repo
lazyadmin run was launched with --cwd inside a project root
```

Medium confidence:

```text
exe/cmdline path is inside known project root
container bind mount points to known project root
```

Low confidence:

```text
parent shell cwd inferred
port convention only
```

#### Project view

The project view answers:

```text
what is running from ~/src/acme?
which ports does it own?
which containers belong to it?
which systemd user services are tied to it?
which lazyadmin runs were launched from this project?
what is orphaned from this project?
```

### 10.6 Special-process classifier

Detect and label common tunnel/forwarder processes:

```text
kubectl port-forward
ssh -L / ssh -R / ssh -D
socat
ngrok
cloudflared
caddy
traefik
minikube tunnel
telepresence
envoy / linkerd-proxy / istio-proxy   (sidecar awareness)
```

These are direct processes but semantically important. Sidecars in particular need to be flagged so users understand the bind ordering when a service mesh is running locally.

Example:

```text
127.0.0.1:8080 -> kubectl port-forward svc/api 8080:80
runtime: direct/kubectl-port-forward
project: inferred from cwd
```

### 10.7 lazyadmin-tracked runtime

#### Objective

Provide a wrapper that puts new dev work into the runtime graph correctly from the start, and gives both humans and AI agents a stable handle for finding, stopping, and inspecting their own processes.

#### Two implementation paths

##### Primary: systemd-run --user --scope (v0.1)

```text
systemd-run --user --scope --unit=lazyadmin-run-<id> --collect -- <cmd>
```

`systemd-run --user --scope` runs the command in a transient systemd scope under the user's session manager. systemd handles cgroup creation, child-subreaper semantics, environment preservation, and lifetime tracking. Stdio goes to the user journal automatically, which solves the historical-stdout problem for tracked runs.

After spawning, lazyadmin records run metadata to:

```text
$XDG_RUNTIME_DIR/lazyadmin/runs/<id>.json
```

containing run ID, tag, command, cwd, env hash, started_at, scope unit name, and creator (user vs agent). The systemd adapter then surfaces the scope as a `LazyadminTracked` workload by recognizing the `lazyadmin-run-` unit prefix.

##### Fallback: manual cgroup v2 scope (v0.2)

For systems without user systemd (alpine, some musl distros, containerized dev environments), a manual path:

```text
1. Fork.
2. Child calls prctl(PR_SET_CHILD_SUBREAPER, 1).
3. Child creates /sys/fs/cgroup/.../lazyadmin.slice/run-<id>.scope/ if writable.
4. Child writes its PID into cgroup.procs.
5. Child redirects stdout/stderr to $XDG_STATE_HOME/lazyadmin/logs/<id>.log.
6. Child execs the target command.
```

This path lands in v0.2. The doctor command reports which path is active.

#### CLI surface

```bash
lazyadmin run [flags] -- <cmd>
  --tag NAME              human-readable tag (default: derived from project + cmd)
  --detach                run in background, return immediately
  --cwd PATH              working directory for the command
  --env KEY=VAL           additional env vars (repeatable)
  --restart-on-exit       systemd Restart=always equivalent (use with care)

lazyadmin runs                          list active tracked runs
lazyadmin runs --json                   structured output
lazyadmin run stop <id|tag>             stop subtree (SIGTERM, then SIGKILL after grace)
lazyadmin run restart <id|tag>          stop + start with same args/env/cwd
lazyadmin run logs <id|tag>             tail captured output (journal or file)
lazyadmin run forget <id|tag>           remove from registry without stopping
```

#### Selector grammar additions

```text
run:<id>                     selects a tracked run by ID
tag:<name>                   selects a tracked run by tag
```

Tags must be unique among active runs. If a tag would collide, the new run gets `<tag>-<short-id>`.

#### Manager-aware stop priority placement

`lazyadmin run stop` slots between systemd actions and raw signals because it's the only path that reliably kills the entire descendant tree of a wrapped command, including children that escaped to a separate session.

#### Logs

Tracked runs always have a log source. systemd-run path: journal filtered by `_SYSTEMD_UNIT=lazyadmin-run-<id>.scope`. Manual path: tail `$XDG_STATE_HOME/lazyadmin/logs/<id>.log`. The "no managed log source" message in section 15.3 now points users at this wrapper as the cure.

#### Registry cleanup

On startup, lazyadmin reconciles `$XDG_RUNTIME_DIR/lazyadmin/runs/` against actual scope state. Entries pointing at scopes that no longer exist are marked `exited` and become `lazyadmin run forget` candidates. A periodic sweep (every refresh interval) keeps the registry honest.

---

## 11. Correlation engine

### 11.1 Snapshot phases

```text
1. Collect host sockets (/proc/net)
2. Collect process table (/proc walk with cache)
3. Map socket inode -> processes (/proc/<pid>/fd, gated by listener inode set)
4. Read tracked-run registry
5. Collect systemd units/socket units (cgroup bulk path + D-Bus targeted)
6. Collect containers (Docker + Podman via bollard)
7. Detect projects
8. Build ownership graph
9. Resolve conflicts and confidence
10. Generate action plans
```

### 11.2 Ownership priority

When multiple sources claim an owner, use ordered evidence, not blind priority.

High-confidence examples:

```text
socket inode found under /proc/<pid>/fd
container API reports published port binding
systemd D-Bus maps PID to unit
Compose canonical labels identify project/service
systemd socket unit ListenStream exactly matches listener
tracked run registry entry matches PID and unit
```

Medium-confidence examples:

```text
cgroup string contains container id
command line resembles known tunnel tool
cwd under project root
container NAT rule suggests mapping but API unavailable
```

Low-confidence examples:

```text
port convention only
parent process cwd only
partial lsof/ss parse without inode
```

### 11.3 Conflict handling

Conflicts are not always bugs.

Examples:

```text
SO_REUSEPORT multiple owners
IPv6 dual-stack bind shadows IPv4
Docker published port plus docker-proxy process
systemd socket unit and activated service both present
same numeric port used in different network namespaces
same port used for TCP and UDP
```

UI behavior:

```text
show all owners
show why each is believed
require explicit selection only when the action is per-owner; for free-port,
  the default is "stop all owners atomically with confirmation" (see 14.3)
```

---

## 12. TUI specification

### 12.1 Default layout

Three-pane layout:

```text
┌ Groups / Filters ───────┬ Workloads / Listeners ──────────────────────────┬ Inspector ──────────────┐
│ All                     │ PORT   BIND        OWNER       RUNTIME    PROJECT│ web                     │
│ Ports                   │ 3000   127.0.0.1   bun         tracked    acme/web│ running                │
│ Public listeners        │ 5173   0.0.0.0     vite        direct     app    │ pid 42420               │
│ Conflicts               │ 5432   127.0.0.1   postgres    compose    localdb│ tag: acme-web           │
│ Orphans                 │ 6379   127.0.0.1   redis       docker     cache  │ cwd ~/src/acme/web      │
│ Tracked runs            │ 8080   [::]        nginx       systemd    -      │ ports: 3000/tcp         │
│ Projects                │                                                   │ logs preview             │
│ Docker / Compose        │                                                   │ actions                  │
│ Podman                  │                                                   │ provenance               │
│ systemd:user            │                                                   │ warnings                 │
│ systemd:system [hidden] │ 12 system listeners hidden. Press S to show.     │                          │
│ Direct processes        │                                                   │                          │
└─────────────────────────┴──────────────────────────────────────────────────┴──────────────────────────┘
```

The "12 system listeners hidden" status line is shown whenever the system-bus filter is active and is hiding rows. It tells users why their `systemctl`-installed Postgres isn't visible in the default view.

### 12.2 Responsive widths

Minimum supported width: 100 columns. Below 100 columns the Inspector collapses into a tab, accessible with `i`. Below 80 columns the layout falls back to single-pane with view switching via `Tab`. The TUI refuses to start below 60 columns and prints a hint to use the CLI.

### 12.3 Views

| View | Purpose |
|---|---|
| Everything | Unified workload/listener table. Default. Two-tier filter applied (see 12.6). |
| Ports | Port-centric view with protocol/address/netns. |
| Projects | Group workloads by repo/project root. |
| Managers | Group by runtime: systemd, Docker, Podman, tracked, direct. |
| Public | Anything not localhost-only. |
| Conflicts | Reused ports, multiple owners, confusing dual-stack cases. |
| Orphans | Processes detached from terminal/session/project parents. |
| Tracked runs | All `lazyadmin run` entries past and present. |
| Logs | Unified log reader. |
| Doctor | Adapter health, permissions, missing capabilities. |

### 12.4 Inspector panel

The right panel shows:

```text
identity
state
runtime
ports/listeners
process tree
project
tracked-run metadata (if applicable)
restart policy
logs preview
warnings
actions
provenance
```

### 12.5 Command palette

Opened with `:`.

Commands:

```text
open
logs
restart
stop
free-port
pause-restart
kill
copy-diagnostic
show-process-tree
show-cgroup
show-network-namespace
edit-unit
edit-compose-file
open-project
refresh
export-json
diff
doctor
toggle-system-services
runs-list
run-stop
run-restart
run-forget
```

### 12.6 Default visibility (two-tier)

The Everything view applies a default filter that hides system-bus units while showing user-bus units, containers, projects, tracked runs, and direct processes. The intent is to surface dev-relevant runtime state without burying it under 30+ system daemon listeners.

Hidden by default:

```text
systemd system-bus units in scope system.slice
kernel-listed daemon listeners with no project/container/run association
known system-managed services (denylist below)
```

Shown by default:

```text
systemd user-bus units (user.slice/[email protected]/...)
Docker / Podman containers
tracked runs (lazyadmin-tracked)
direct processes with project association
direct processes with no project association but in user UID
```

Default system-service denylist (config-overridable):

```text
systemd-resolved
systemd-networkd
systemd-timesyncd
systemd-logind
systemd-udevd
NetworkManager
dbus-daemon
avahi-daemon
cups / cups-browsed
chronyd / ntpd
sshd
fwupd
ModemManager
gdm / sddm / lightdm
polkitd
rtkit-daemon
```

#### Bypass rules

Point queries always bypass the filter. `lazyadmin :PORT`, `lazyadmin pid:N`, `lazyadmin unit:foo.service`, `lazyadmin container:bar` all return the truth without filtering. The reasoning: if a user is asking about a specific entity, they want ground truth, not a filtered subset.

The TUI toggles the filter with `S` (capital S). The filter state shows in the Groups pane as `systemd:system [hidden]` or `systemd:system`. When the filter is hiding rows, the Workloads pane shows a status line with the hidden count.

### 12.7 Keybindings

| Key | Action |
|---|---|
| `/` | Fuzzy filter current list |
| `:` | Command palette |
| `Tab` | Next pane |
| `Shift+Tab` | Previous pane |
| `Enter` | Inspect / expand selected entity |
| `l` | Logs |
| `p` | Ports owned by selected entity |
| `t` | Process tree |
| `r` | Restart via safest manager-aware method |
| `s` | Stop via safest manager-aware method |
| `f` | Free selected port |
| `k` | Kill process / process group after confirmation |
| `o` | Open local URL |
| `e` | Edit source config when safe |
| `y` | Copy diagnostic summary |
| `S` | Toggle system-services visibility |
| `R` | Run a new command (`lazyadmin run` interactive) |
| `?` | Help |
| `q` | Quit |

### 12.8 Fuzzy search

Search matches:

```text
port
bind address
process name
command
cwd
project name
container name
compose service
systemd unit
image name
runtime kind
tracked run tag
```

### 12.9 Warning badges

```text
PUBLIC       bound to 0.0.0.0, ::, LAN, or published beyond localhost
CONFLICT     multiple owners or ambiguous dual-stack bind
ROOT         owner requires elevated permission for details/actions
SOCKET-ACT   systemd socket activation owns listener
ORPHAN       parent/session/project no longer active
STALE        process from deleted executable or old cwd
TUNNEL       ssh/kubectl/socat/cloudflared/ngrok/sidecar
TRACKED      managed by lazyadmin run
RESTARTING   restart policy will re-bind on stop
```

#### Definitions

`ORPHAN`: the process's session leader (sid) has exited, or the process has been reparented to PID 1 and has no project/manager association. We pick session-based orphan detection because it's stable and cheap to compute.

---

## 13. CLI specification

### 13.1 Commands

```bash
lazyadmin                         # TUI
lazyadmin :3000                   # explain port 3000 (full provenance + actions)
lazyadmin :3000 --brief           # one-line scripting output
lazyadmin port 3000               # explicit port query (same as :3000)
lazyadmin free 3000               # guided free-port action
lazyadmin ps                      # compact workload list
lazyadmin ps --json               # JSON workload list
lazyadmin public                  # public/LAN listeners
lazyadmin conflicts               # ambiguous or conflicting listeners
lazyadmin projects                # project-grouped output
lazyadmin logs <selector>         # manager-aware logs
lazyadmin doctor                  # adapter/permission health
lazyadmin doctor --json           # structured health output
lazyadmin export --json           # full graph snapshot
lazyadmin diff <before> <after>   # snapshot diff (file paths or `-` for stdin/current)

lazyadmin run [flags] -- <cmd>    # spawn a tracked run
lazyadmin runs                    # list tracked runs
lazyadmin runs --json
lazyadmin run stop <id|tag>
lazyadmin run restart <id|tag>
lazyadmin run logs <id|tag>
lazyadmin run forget <id|tag>

lazyadmin pause-restart <selector>     # disable restart policy until next start
lazyadmin resume-restart <selector>    # re-enable restart policy
```

The previously-separate `explain` command is merged into the bare port query. The default output of `lazyadmin :PORT` shows identity, owner, warnings, full provenance, and actions. `--brief` produces one line for shell scripting:

```text
$ lazyadmin :3000 --brief
tcp/127.0.0.1:3000 owner=bun(42420) project=acme/web runtime=tracked confidence=high
```

### 13.2 Selector grammar

RFC 3986 authority syntax for hosts. IPv6 literals require brackets.

```text
:3000                                bare port, any address, default protocol
127.0.0.1:3000                       IPv4 host:port
[::1]:3000                           IPv6 host:port
[::]:3000                            IPv6 any
[fe80::1]:3000                       IPv6 link-local
tcp/:3000                            protocol prefix on any
tcp/127.0.0.1:3000                   protocol prefix on IPv4
tcp/[::1]:3000                       protocol prefix on IPv6
udp/[::]:5353                        UDP IPv6 any
unix:///tmp/app.sock                 unix socket
pid:42420                            process selector
unit:dev-api.service                 systemd service unit
unit:dev-api.socket                  systemd socket unit (parser routes by suffix)
container:localdb-postgres-1         container by name
container:abc123def                  container by ID prefix
compose:localdb/postgres             compose project/service
project:acme/web                     project by config name
project:~/src/acme/web               project by absolute path
run:r-7f9a                           tracked run by ID
tag:acme-web                         tracked run by tag
```

Bracketed IPv6 without a port is rejected: `[::1]` alone is ambiguous (host or listener?), so the parser requires `[::1]:PORT`.

### 13.3 Example output

```text
$ lazyadmin :5432

tcp://0.0.0.0:5432

Owner:
  Compose project: localdb
  Service: postgres
  Container: localdb-postgres-1
  Runtime: docker-compose
  Mapping: 0.0.0.0:5432 -> 5432/tcp
  Restart policy: unless-stopped

Warning:
  Published beyond localhost. Reachability depends on host firewall/routing.

Why lazyadmin believes this:
  ✓ container API reports published binding 0.0.0.0:5432/tcp
  ✓ container label com.docker.compose.project=localdb
  ✓ container label com.docker.compose.service=postgres

Actions:
  o open if HTTP-ish
  l logs
  s stop compose service
  r restart compose service
  f free port (will stop the container; restart policy may re-bind)
  pause-restart  disable auto-restart, then stop
  y copy diagnostic
```

---

## 14. Actions specification

### 14.1 Action type

```rust
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub kind: ActionKind,
    pub danger: DangerLevel,
    pub requires: Vec<Requirement>,
    pub dry_run: Vec<String>,
    pub target: EntityRef,
    pub confirmation: ConfirmationPolicy,
}
```

### 14.2 Danger levels

```text
safe          open URL, copy diagnostic, view logs, list runs
warn          restart service/container, stop dev service, pause-restart
destructive   kill process, stop system service, remove binding, SIGKILL
```

### 14.3 Free-port algorithm

```text
Input: selected listener or numeric port

1. Resolve exact listener set:
   protocol, address, namespace, port

2. Resolve all owners:
   tracked runs
   compose services
   containers (Docker, Podman)
   systemd services
   systemd sockets
   direct processes

3. Build a unified action plan covering ALL owners:
   one entry per owner, each with its preferred manager-aware method

4. Show consolidated dry run:
   every owner that will be stopped
   which ports will disappear
   restart policies in effect (if any)
   what will not be touched

5. Single confirmation for the entire set.

6. Execute in parallel:
   each per-owner action runs concurrently
   collect per-owner result (success / error / timeout)

7. Verify factually:
   rescan the listener
   diff against pre-action snapshot
   report what changed without judging success/failure

8. If listener still bound:
   show the new owner (often a manager-driven restart)
   offer pause-restart as a follow-up action
   never auto-escalate to SIGKILL
```

### 14.4 Confirmation examples

#### Single direct process

```text
Free tcp://127.0.0.1:3000?

Owner:
  bun run dev
  PID: 42420
  Process group: 42420
  CWD: ~/src/acme/web
  Parent: tmux -> zsh

Planned action:
  Send SIGTERM to process group 42420
  Wait for listener to disappear

Type "free" to continue:
```

#### Multi-owner

```text
Free tcp://0.0.0.0:5432?

Will stop 3 owners:
  1. Compose service localdb/postgres
     (container localdb-postgres-1, restart policy unless-stopped)
  2. systemd user unit dev-cache.service
     (restart policy on-failure)
  3. Direct process: redis-server
     (PID 19234, ~/src/scratch)

Restart policies will be in effect after stop.
Use `pause-restart` first to make stops stick.

Type "free" to stop all 3, or "pause-and-free" to disable restart first:
```

#### systemd socket

```text
Stop systemd socket dev-api.socket?

This socket owns:
  [::]:8080

The service dev-api.service is currently inactive.
Incoming traffic would activate it.

Type "stop socket" to continue:
```

### 14.5 Verify reporting

After any free/stop/restart action, lazyadmin runs a fresh listener scan and produces a factual diff:

```text
Action complete.
  ✓ Stopped: Compose service localdb/postgres
  ✓ Stopped: systemd user unit dev-cache.service
  ✗ Failed: redis-server (SIGTERM sent, did not exit within 5s)

Listener tcp://0.0.0.0:5432:
  Before: container localdb-postgres-1 (PID 19234)
  After:  container localdb-postgres-1 (PID 19891)

The container was restarted automatically. Restart policy: unless-stopped.

Next steps:
  pause-restart compose:localdb/postgres   disable auto-restart
  free 5432                                retry after pausing restart
  run stop tag:scratch-redis               stop the still-running owner
```

No inference about whether this is "success" or "failure": the action plan executed, and here's what changed. The user decides next steps.

### 14.6 pause-restart action

A new action specifically for managers with restart policies. Disables the auto-restart so a subsequent stop sticks.

For Docker / Podman:

```text
POST /containers/{id}/update with HostConfig.RestartPolicy.Name=no
```

For systemd:

```text
systemctl mask <unit>      followed by stop
```

The change is reversible:

```bash
lazyadmin resume-restart <selector>
```

The doctor command lists currently-paused restart policies so users don't forget they masked a unit.

---

## 15. Logs specification

### 15.1 Unified log sources

| Runtime | Preferred source | Fallback |
|---|---|---|
| systemd | journal API | `journalctl` |
| Docker | bollard logs API | `docker logs` |
| Docker Compose | container logs grouped by service | `docker compose logs` |
| Podman | bollard logs API (read-only in v0.1) | `podman logs` |
| lazyadmin-tracked (systemd path) | journal filtered by scope unit | `journalctl --user-unit=lazyadmin-run-<id>.scope` |
| lazyadmin-tracked (manual path, v0.2) | tail captured log file | n/a |
| Direct process | unavailable unless wrapped | suggest `lazyadmin run` |

### 15.2 Log viewer requirements

```text
follow mode
pause/resume
tail N
filter text
copy selected line
jump to latest
show source labels
preserve ANSI optionally
JSON log parsing when available
```

### 15.3 Direct-process caveat

For a raw dev server started from a shell without `lazyadmin run`, lazyadmin usually cannot recover historical stdout/stderr.

```text
No managed log source found.
This process was started directly from a shell.

To capture logs in the future, restart it under lazyadmin run:
  lazyadmin run --tag <name> -- <your command>

To view logs going forward only, you can attach to its current FDs
if they point to a regular file (use --attach if applicable).
```

For processes whose `/proc/<pid>/fd/1` or `/proc/<pid>/fd/2` resolves to a regular file (someone redirected `> /tmp/dev.log`), lazyadmin offers a `tail-file` action that follows that file. Cheap, safe, and covers a real percentage of cases without strace.

---

## 16. Security and privacy

### 16.1 Default permission posture

```text
run unprivileged
read what current user can read
show permission gaps
escalate only for targeted rediscovery/action
never install setuid helper by default
never run root daemon by default
```

### 16.2 Secret redaction

Redact by default from:

```text
cmdline
environ
container env
systemd Environment=
compose env
```

#### Pattern matching

Variable name patterns:

```text
token
secret
password
passwd
pwd
apikey
api_key
authorization
credential
session
cookie
private_key
```

URL userinfo: any `scheme://user:pass@host` pattern in cmdline or env values is redacted to `scheme://user:<redacted>@host`. This catches the very common case of Postgres / Redis / Mongo connection strings appearing in dev server cmdlines.

Display:

```text
DATABASE_PASSWORD=<redacted>
--token=<redacted>
DATABASE_URL=postgresql://app:<redacted>@localhost/db
```

Allow explicit reveal only with confirmation.

### 16.3 Container socket warning

Doctor output warns when the user has direct Docker socket access:

```text
Docker socket accessible.
This usually grants root-equivalent control of the host.
```

### 16.4 Public exposure warning

Warn on:

```text
0.0.0.0 binds
:: binds
specific non-loopback interface binds
container published ports not bound to loopback
systemd sockets listening on all interfaces
```

Phrase carefully:

```text
reachable beyond localhost depending on firewall/routing
```

### 16.5 Tracked-run security

`lazyadmin run` does not run as root by default. systemd-run inherits the user's permissions. The wrapper does not capture passwords, keys, or token args (they're redacted in the registry the same as elsewhere). The registry itself is mode 0700 under `$XDG_RUNTIME_DIR`.

---

## 17. Configuration

Use TOML.

Config path:

```text
$XDG_CONFIG_HOME/lazyadmin/config.toml
~/.config/lazyadmin/config.toml
```

Example:

```toml
[ui]
refresh_interval_ms = 1500
default_view = "everything"
hide_system_services = true       # two-tier filter; press S to toggle in TUI
show_kernel_listeners = false
mouse = true

[ports]
common = [3000, 3001, 5173, 5432, 6379, 8000, 8080, 9000]
warn_public_binds = true
include_udp = true
include_unix = true
include_established = false

[actions]
default_signal = "TERM"
kill_process_group = true
require_type_to_confirm = true
sudo_mode = "on-demand"
verify_after_action = true
free_multi_owner = "stop_all"     # stop_all | prompt | refuse

[redaction]
enabled = true
patterns = [
  "token",
  "secret",
  "password",
  "apikey",
  "authorization",
  "credential",
]
url_userinfo = true               # redact scheme://user:pass@host

[adapters.sockets]
enabled = true
preferred = "proc"                # v0.1: "proc". v0.2 will accept "sock_diag".
fallbacks = ["ss"]

[adapters.systemd]
enabled = true
user = true
system = true
bulk_via_cgroup = true            # use /proc/<pid>/cgroup for bulk unit inference

[adapters.container]
enabled = true
docker_socket = "auto"            # auto | $DOCKER_HOST | path
podman_rootless = true
podman_rootful = false
show_stopped = false
inspect_lazy = true               # only call /containers/{id}/json on selection

[adapters.tracked]
enabled = true
spawn_method = "auto"             # auto | systemd_run | manual_cgroup
log_dir = "$XDG_STATE_HOME/lazyadmin/logs"

[projects]
roots = ["~/src", "~/code", "~/work"]
markers = [
  ".git",
  "package.json",
  "pyproject.toml",
  "Cargo.toml",
  "go.mod",
  "compose.yaml",
  "flake.nix",
  ".envrc",
]

[visibility.system_service_denylist]
units = [
  "systemd-resolved.service",
  "systemd-networkd.service",
  "systemd-timesyncd.service",
  "systemd-logind.service",
  "systemd-udevd.service",
  "NetworkManager.service",
  "dbus.service",
  "avahi-daemon.service",
  "cups.service",
  "chronyd.service",
  "sshd.service",
  # plus more by default; see section 12.6
]
```

---

## 18. JSON output

### 18.1 Snapshot

```bash
lazyadmin export --json
```

Produces:

```json
{
  "schema_version": "lazyadmin.snapshot.v1",
  "generated_at": "2026-04-27T12:00:00Z",
  "host": {
    "boot_id": "...",
    "hostname": "...",
    "kernel": "..."
  },
  "managers": [],
  "processes": [],
  "listeners": [],
  "workloads": [],
  "projects": [],
  "tracked_runs": [],
  "edges": [],
  "warnings": []
}
```

`tracked_runs` is a top-level array of run registry entries, separate from `workloads` so consumers can find runs even when their underlying scope has exited.

### 18.2 Diff

```bash
lazyadmin diff <before> <after>
```

`<before>` and `<after>` are paths to snapshot JSON files, or `-` for stdin / current state.

```bash
# typical use
lazyadmin export --json > /tmp/before.json
# ... make changes ...
lazyadmin diff /tmp/before.json -
```

Output (human-readable by default, `--json` for structured):

```text
Listeners:
  + tcp/127.0.0.1:3000 (bun, project acme/web, tracked run r-7f9a)
  - tcp/127.0.0.1:3001 (vite, project app)
  ~ tcp/0.0.0.0:5432 owner changed
      before: container localdb-postgres-1 (PID 19234)
      after:  container localdb-postgres-1 (PID 19891)

Workloads:
  + tracked run r-7f9a (tag: acme-web)
  - direct process redis-server (PID 19234)
```

Powers the verify step (section 14.5) and the agent-friendly "what changed" workflow.

### 18.3 Diagnostic copy

`y` or `copy-diagnostic` copies compact Markdown:

```text
lazyadmin diagnostic

listener: tcp://127.0.0.1:3000
owner: web
runtime: lazyadmin-tracked
tag: acme-web
run_id: r-7f9a
pid: 42420
cmd: bun run dev
cwd: ~/src/acme/web
project: acme/web
confidence: high

provenance:
- /proc/net/tcp listener inode 123456
- /proc/42420/fd/17 -> socket:[123456]
- cwd under git root ~/src/acme/web
- tracked-run registry entry r-7f9a
```

---

## 19. Doctor command

### 19.1 Output format

`lazyadmin doctor` produces structured results internally (severity + summary + hint) rendered as text by default, JSON with `--json`, and a status board in the TUI.

### 19.2 Checks

```text
OS and kernel
/proc readable
/proc/net readable
ss available
systemd system bus reachable
systemd user bus reachable
systemd-run --user available (lazyadmin run primary path)
journal readable
container socket reachable (Docker)
container socket reachable (Podman rootless)
container socket reachable (Podman rootful)
container API version negotiation
container socket permission risk
project roots exist
terminal capabilities
clipboard availability
NO_COLOR honoring
tracked-run registry directory writable
masked units (paused-restart leftovers)
```

### 19.3 Example

```text
$ lazyadmin doctor

sockets:
  /proc/net: ok (primary)
  ss fallback: ok
  sock_diag: not used in v0.1

processes:
  /proc: ok
  unreadable processes: 12 root-owned

systemd:
  user bus: ok
  system bus: ok, privileged actions require polkit/sudo
  journal: partial, system journal not readable
  bulk cgroup path: ok

tracked runs:
  systemd-run --user: ok (primary path)
  manual cgroup fallback: not active
  registry: $XDG_RUNTIME_DIR/lazyadmin/runs/ ok
  active runs: 2
  exited runs awaiting forget: 1

containers:
  Docker socket: ok, API 1.52
  Podman rootless socket: not found
  Podman rootful socket: not checked by config
  warning: docker socket access is root-equivalent

paused-restart units (won't auto-restart until resumed):
  compose:localdb/postgres
  systemd:user dev-cache.service
```

---

## 20. Edge cases and required behavior

| Edge case | Required behavior |
|---|---|
| systemd socket active, service inactive | Show `.socket` as owner; do not invent a PID. |
| Container published port with no host PID | Show binding from container API. |
| docker-proxy process appears | Correlate to container binding; avoid double-counting. |
| IPv6 `[::]:3000` may also serve IPv4 | Show dual-stack warning based on `IPV6_V6ONLY` socket option. |
| `0.0.0.0` bind | Mark as beyond-localhost; do not overclaim internet exposure. |
| `SO_REUSEPORT` | Show multiple owners; default free-port action stops all. |
| Same port TCP and UDP | Show protocol clearly. |
| Same port in host and container namespaces | Treat as separate listeners. |
| Permission denied under `/proc` | Show partial row and permission reason. |
| PID reused during scan | Use ProcessKey (pid + boot_id + start_time); rescan validation. |
| Process from deleted executable | Show `(deleted)` and still use cwd/project. |
| Terminal died but child lives | Mark as orphan when sid leader exited. |
| Shell script wrapper | Show process tree, not just shell. |
| Raw process logs unavailable | Say so plainly; suggest wrapping in `lazyadmin run`. |
| Rootless Podman | Use user socket via bollard; mark rootless. |
| Docker Desktop / WSL2 | Detect via daemon info; container netns is invisible from host; rely on container API for binds. |
| kubectl/ssh/socat/cloudflared/ngrok | Label as tunnel/forwarder. |
| Service mesh sidecar (envoy, linkerd-proxy, istio-proxy) | Label as sidecar; explain that the published port belongs to the proxy, not the app. |
| `npm run dev` spawning into a separate session | Tracked-run path captures the subtree via systemd scope; without wrapping, free-port may need "kill subtree" rather than PGID kill. |
| Tracked run scope exits unexpectedly | Mark registry entry `exited`; offer `forget`. |
| Unit masked by pause-restart | Surface in Doctor; reflect in workload state. |

---

## 21. MVP scope

### 21.1 Must ship in MVP (v0.1, target ~5 months)

```text
Linux only
Rust core
Ratatui TUI
CLI subcommands
TCP/UDP IPv4/IPv6 listeners
Unix socket visibility
/proc/net primary socket discovery
process enrichment through /proc with caching
socket inode -> PID mapping with gated FD walk
systemd system + user unit correlation (cgroup bulk + D-Bus targeted)
systemd socket unit handling
restart policy detection (Docker, Podman, systemd)
container adapter via bollard (Docker + Podman discovery)
Compose grouping via canonical labels
Docker logs and stop/restart/pause-restart actions
Podman read-only discovery and inspection (actions in v0.2)
project root detection
lazyadmin run wrapper (systemd-run path)
manager-aware free-port action with multi-owner stop-all
public listener warnings
conflict view
orphan view
tracked-runs view
doctor command with structured output
JSON snapshot export
JSON diff export
config file
redaction including URL userinfo
copy diagnostic
two-tier visibility filter
agent skill (lazyadmin-agent) shipped alongside binary
```

### 21.2 Should ship soon after MVP (v0.2)

```text
Podman actions
Podman logs follow
Podman pods (Libpod) UI
sock_diag optimized socket discovery path
manual cgroup fallback for lazyadmin run (no systemd required)
systemd journal richer integration (sd-journal bindings)
process tree visualization
metrics panel
container events live updates
systemd D-Bus PropertiesChanged signal updates
configurable keybindings
themes
Nix flake / Homebrew tap
direct-process log tail-file support
```

### 21.3 Explicitly post-MVP

```text
macOS launchd adapter
Kubernetes API integration
full Windows support
eBPF event tracing
web dashboard
remote host management
plugin protocol
automatic remediation policies
```

---

## 22. Repo layout

```text
lazyadmin/
  Cargo.toml
  crates/
    lazyadmin-core/
      src/
        model/
        graph/
        snapshot/
        diff/
        correlate/
        actions/
        config/
        redact/
    lazyadmin-tui/
      src/
        app/
        views/
        widgets/
        keymap/
        command_palette/
    lazyadmin-cli/
      src/
    lazyadmin-adapter-procfs/
    lazyadmin-adapter-sockets/
    lazyadmin-adapter-systemd/
    lazyadmin-adapter-container/    # Docker AND Podman via bollard
    lazyadmin-adapter-project/
    lazyadmin-adapter-tracked/      # lazyadmin run runtime
  testdata/
    procfs/
    sockets/
    systemd/
    container/
    tracked/
  skills/
    lazyadmin-agent/                # shipped with binary, see section 31
  docs/
    spec.md
    adapter-protocol.md
    action-safety.md
    troubleshooting.md
    agent-integration.md
```

### 22.1 Crate responsibilities

```text
lazyadmin-core
  owns model, graph, correlation, actions, redaction, JSON, diff

lazyadmin-cli
  owns command parsing and non-TUI output

lazyadmin-tui
  owns Ratatui app state, views, widgets, keybindings

adapter crates
  provide discovery / action / log capabilities with stable contracts
```

---

## 23. Adapter trait design

```rust
#[async_trait]
pub trait DiscoveryAdapter {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;
    async fn health(&self) -> AdapterHealth;
    async fn discover(&self, ctx: DiscoveryContext) -> Result<DiscoveryOutput>;

    /// Returns an event stream for incremental updates, or None if the adapter
    /// only supports polling. v0.1 adapters all return None; v0.2 adds streams
    /// for container events and systemd D-Bus signals.
    async fn watch(&self) -> Option<BoxStream<'static, DiscoveryEvent>> {
        None
    }
}

pub struct DiscoveryOutput {
    pub managers: Vec<Manager>,
    pub processes: Vec<Process>,
    pub listeners: Vec<Listener>,
    pub workloads: Vec<Workload>,
    pub projects: Vec<Project>,
    pub tracked_runs: Vec<TrackedRun>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<Warning>,
}

pub enum DiscoveryEvent {
    Added(Entity),
    Removed(EntityRef),
    Updated(Entity),
    HealthChanged(AdapterHealth),
}
```

The graph engine accepts either a full `DiscoveryOutput` (snapshot replace) or a stream of `DiscoveryEvent` (incremental). v0.1 always replaces. v0.2 starts mixing in events for adapters that support it.

Action adapter:

```rust
#[async_trait]
pub trait ActionExecutor {
    fn name(&self) -> &'static str;
    async fn plan(&self, target: EntityRef, graph: &Graph) -> Result<Vec<Action>>;
    async fn execute(&self, action: Action, ctx: ActionContext) -> Result<ActionResult>;
}
```

Log adapter:

```rust
#[async_trait]
pub trait LogProvider {
    fn name(&self) -> &'static str;
    async fn can_stream(&self, target: EntityRef, graph: &Graph) -> bool;
    async fn stream(&self, target: EntityRef, opts: LogOptions) -> Result<LogStream>;
}
```

---

## 24. Performance requirements

Target behavior on a typical developer workstation:

```text
initial snapshot under 1s for common cases
refresh under 300ms after caches warm
TUI input latency under 50ms
no blocking UI during container or systemd API timeouts
adapter timeouts configurable
```

Implementation notes:

```text
run adapters concurrently
cache stable process metadata by ProcessKey
cache project roots
gate /proc/<pid>/fd readlinks by listener-inode set
diff snapshots conceptually instead of redrawing everything
rate-limit expensive scans
make container API calls cancellable
do not block UI on per-container inspect (lazy on selection)
separate UI event loop from discovery tasks
```

---

## 25. Testing strategy

### 25.1 Unit tests

```text
/proc parsers
/proc/net parsers
socket address decoding
redaction (including URL userinfo)
selector parsing (including bracketed IPv6)
project detection
correlation rules
action planning
diff computation
multi-owner free planning
```

### 25.2 Fixture tests

```text
testdata/procfs/simple-node-listener
testdata/procfs/reuseport-multiple-pids
testdata/procfs/permission-denied
testdata/container/compose-postgres
testdata/container/podman-rootless
testdata/systemd/socket-activation
testdata/systemd/restart-always
testdata/tracked/active-scope
testdata/tracked/exited-scope
```

### 25.3 Integration tests

Run in Linux CI when possible:

```text
spawn local TCP listener
spawn UDP listener
spawn systemd user service if environment supports it
run a container with published localhost port
run a container with 0.0.0.0 published port
run a Compose project
launch lazyadmin run, observe in tracked-runs view, stop it
verify lazyadmin export --json
verify lazyadmin diff
```

### 25.4 Golden snapshot tests

Use `insta` for snapshot tests of view-model output, not raw terminal pixels. View models are JSON-shaped and stable; rendered terminal output has too many control sequences to compare reliably.

### 25.5 Safety tests

```text
free-port chooses Docker stop over SIGKILL of docker-proxy
free-port chooses systemd StopUnit over SIGTERM of service main PID
free-port stops all owners atomically and reports per-owner result
systemd socket action stops socket when service inactive
multi-owner free dry run enumerates every owner
permission denied does not hide rows
redaction catches common secret patterns
URL userinfo redaction catches DATABASE_URL forms
verify reports auto-restart factually without claiming success or failure
pause-restart followed by free actually frees the port
tracked run stop terminates the entire descendant tree
```

---

## 26. Packaging and distribution

MVP distribution:

```text
GitHub releases with Linux x86_64/aarch64 binaries
cargo install lazyadmin
Nix flake
Homebrew tap (v0.2)
```

Packaging goals:

```text
single binary where possible
no mandatory root install
no mandatory daemon
works in tmux/ssh
works in minimal terminals (graceful below 100 cols)
agent skill shipped in package and published separately
```

The agent skill ships as a tarball asset on each release: `lazyadmin-agent-skill-v<version>.tar.gz`, plus an installation script that drops it into common skill directories.

---

## 27. Implementation milestones

### Milestone 0 — Core model and CLI skeleton

```text
workspace setup
core model (entities, edges, provenance, ProcessKey)
config loading
selector parser (with IPv6 brackets, run/tag selectors)
JSON snapshot format
diff format
basic CLI command routing
```

### Milestone 1 — Socket/process engine

```text
/proc process scan with ProcessKey caching
/proc/net listener parsing
socket inode -> PID mapping with gated FD walk
process tree
project detection v0
lazyadmin :PORT output (merged bare + explain)
lazyadmin --brief flag
```

### Milestone 2 — systemd

```text
system/user bus health
PID -> unit correlation (cgroup bulk + D-Bus targeted)
unit list
socket unit parsing
restart policy detection
StopUnit/RestartUnit actions
journal logs fallback
```

### Milestone 3 — Tracked runtime

```text
systemd-run --user --scope spawn
run registry under $XDG_RUNTIME_DIR
tracked-run discovery (joins with systemd adapter)
lazyadmin run / runs / run stop / run logs
manager-aware action priority slot
"no log source" message updated
```

### Milestone 4 — Container adapter

```text
bollard wired for Docker + Podman
socket discovery
container list / inspect (lazy)
published port mapping
Compose label parsing
restart policy detection
logs (follow)
stop/restart actions (Docker)
public bind warning
pause-restart action
Podman read-only verified
```

### Milestone 5 — Ratatui MVP

```text
three-pane layout with responsive fallback
Everything view with two-tier filter
Ports view
Tracked runs view
Inspector
search/filter
refresh
copy diagnostic
S to toggle system services
R to spawn lazyadmin run interactively
```

### Milestone 6 — Free-port workflow

```text
multi-owner action planner
consolidated dry run
parallel execute with per-owner reporting
factual verify with diff
escalation paths (pause-restart, kill-subtree, sudo retry)
confirmation modals
```

### Milestone 7 — Polish, doctor, agent skill

```text
doctor command with structured output
agent skill v1 (SKILL.md + cheatsheet + examples)
configurable keybindings
themes minimal
packaging
CI fixtures
docs
release v0.1
```

---

## 28. Acceptance criteria for v0.1

`lazyadmin` v0.1 is acceptable when the following are true:

```text
1. lazyadmin :3000 explains the owner of a direct TCP listener with full provenance.
2. lazyadmin :5432 explains a Compose-published Postgres port and the restart policy.
3. systemd user and system services are shown distinctly.
4. systemd socket activation is represented without requiring a live service PID.
5. Public/non-loopback listeners are easy to find.
6. A Compose service can be stopped from the TUI with confirmation.
7. A direct process can be terminated with SIGTERM and verified.
8. Multiple owners on a single port never collapse into one fake owner.
9. free 5432 with 3 owners stops all 3 atomically and reports per-owner result.
10. Permission-denied information is visible, not silently hidden.
11. JSON export includes listeners, processes, workloads, managers, projects, tracked runs, edges, and provenance.
12. JSON diff produces meaningful before/after for listener and workload changes.
13. Doctor gives actionable adapter/permission status with structured severity.
14. No secret-looking environment values, including URL userinfo, are shown by default.
15. lazyadmin run wraps a command, the wrapped process appears in the runtime graph, and run stop terminates the entire subtree including grandchildren.
16. Verify after free-port reports factually when a manager auto-restarts the unit; never claims success or failure.
17. Two-tier filter hides system-bus units by default; point queries bypass the filter.
18. Bracketed IPv6 selectors parse correctly: lazyadmin "tcp/[::1]:3000".
19. The lazyadmin-agent skill ships in the release artifact and installs cleanly into a coding agent's skill directory.
```

---

## 29. Future plugin protocol

Post-MVP, allow external adapters:

```bash
lazyadmin-adapter-supervisor discover --json
lazyadmin-adapter-supervisor actions --entity <id> --json
lazyadmin-adapter-supervisor logs --entity <id>
```

Protocol:

```json
{
  "adapter": "supervisor",
  "version": "1.0.0",
  "capabilities": ["discover", "actions", "logs"],
  "schema_version": "lazyadmin.adapter.v1"
}
```

Plugins run as the user; lazyadmin does not grant privilege escalation to plugins. Plugin actions are subject to the same confirmation policies as built-in actions.

Candidate adapters:

```text
supervisord
pm2
foreman/overmind
honcho
launchd
Kubernetes port-forward (graduating from special-process classifier to full adapter)
Nomad dev agent
Caddy/Traefik local reverse proxy
```

---

## 30. macOS adapter outline (post-MVP)

macOS is a separate adapter, not Linux semantics in costume.

Use:

```text
lsof or netstat equivalent for sockets
launchctl for launchd jobs
plist discovery
process inspection through libproc where possible
```

Expected support:

```text
port -> pid -> process
pid -> launchd label if available
launchd label -> plist path
launchd action -> stop/start/kickstart/bootout equivalent
```

`lazyadmin run` on macOS would use a manual cgroup-equivalent (process group + child-subreaper) rather than systemd-run. The wrapper API is the same; the implementation is platform-specific.

---

## 31. Agent integration

### 31.1 Goal

AI coding agents that spawn dev servers, build watchers, and test harnesses on behalf of users need a stable substrate. Without one they fall back to `kill $(lsof -ti :PORT)`, orphan databases, and lose track of their own children. `lazyadmin` plus a shipped agent skill turns this into a first-class capability.

### 31.2 The skill

Ships at `skills/lazyadmin-agent/` in the repo and as a release artifact. Structure:

```text
lazyadmin-agent/
  SKILL.md                   trigger description + entry point
  always-do-this.md          behavioral rules
  cheatsheet.md              compact command reference (token-efficient)
  json-schema-v1.md          documented snapshot fields the agent will use
  examples/
    01-start-dev-server.md
    02-port-conflict.md
    03-find-my-process.md
    04-tail-logs.md
    05-snapshot-diff.md
    06-fallback-no-lazyadmin.md
```

### 31.3 SKILL.md trigger description

```text
Manage local development runtime state on Linux dev machines via lazyadmin.
Use this skill whenever the agent is about to:
- start a long-running dev server, build watcher, or background process
- diagnose a port conflict, EADDRINUSE error, or "address already in use"
- check what's currently running, what owns a port, or what a service is doing
- stop, restart, or free a port from a previous run
- investigate why a local URL is unreachable or a service appears stuck
- check whether a previously-started service is still alive
Also trigger proactively when the user mentions a service being broken, stuck,
or "still running from earlier." This skill is Linux-specific; on macOS the
agent should fall back to traditional tools.
```

### 31.4 Behavioral rules (always-do-this.md)

```text
1. Check availability first. Run `command -v lazyadmin && lazyadmin doctor --json`
   once at the start of a session. If absent or unhealthy, fall back to ss/lsof
   and standard signals. Note the absence so subsequent answers don't assume
   lazyadmin is present.

2. Never run `npm run dev`, `bun run dev`, `pnpm dev`, `cargo run`,
   `python -m http.server`, or similar long-running commands directly. Wrap them:

     lazyadmin run --tag <descriptive> --detach -- <cmd>

   The tag should describe the project/role: --tag acme-api, --tag worker,
   --tag db-proxy. This is how you and the user find the process later.

3. Never use `kill $(lsof -ti :PORT)`, `fuser -k`, or `pkill -f`. These are
   the patterns that orphan databases and corrupt state. Instead:

     lazyadmin :PORT              # see what's there
     lazyadmin free PORT          # safe, manager-aware free with confirmation

4. Diagnose with structured output. For programmatic checks, prefer:

     lazyadmin :PORT --json
     lazyadmin runs --json
     lazyadmin export --json

   Never parse lazyadmin's human-readable output.

5. After mutations that affect the runtime graph (starting/stopping services,
   running migrations, restarting containers), capture a diff:

     lazyadmin export --json > /tmp/before.json
     <do thing>
     lazyadmin diff /tmp/before.json -

   Useful for explaining to the user what actually changed.

6. When stopping work, list and stop your own runs:

     lazyadmin runs --json | jq '.runs[] | select(.tag | startswith("agent-"))'
     lazyadmin run stop tag:<name>

   Do not leave wrappers running across sessions unless the user asked you to.

7. When the user asks "is X running?" or "what's on port Y?", do not guess
   from memory of earlier in the conversation. Re-query lazyadmin. State
   changes between turns.
```

### 31.5 JSON schema documentation (json-schema-v1.md)

Documents the fields the agent will commonly need:

```text
listeners[].port, .bind_addr, .protocol, .exposure, .owners[]
listeners[].owners[].workload_id  -> workloads[].id
workloads[].runtime               -> Lazyadmin / Docker / DockerCompose / SystemdUser / ...
workloads[].pids[].pid
workloads[].project.name
workloads[].project.root
workloads[].state                 -> Running / Stopped / Restarting / Unknown
workloads[].lazyadmin_run_id      -> present iff started via lazyadmin run
workloads[].restart_policy
workloads[].actions[]             -> available actions for this workload
tracked_runs[].id, .tag, .cmd, .cwd, .started_at, .state
warnings[]                        -> public binds, conflicts, permission gaps
```

With concrete `jq` recipes:

```bash
# What's on port 3000?
lazyadmin export --json | jq '.listeners[] | select(.port==3000)'

# What did I start as an agent?
lazyadmin export --json | jq '.workloads[] | select(.runtime=="LazyadminTracked")'

# Anything bound publicly?
lazyadmin export --json | jq '.listeners[] | select(.exposure=="LanOrPublic" or .exposure=="Public")'

# Did the migration break a listener?
lazyadmin diff /tmp/before.json - --json | jq '.removed.listeners[]'
```

### 31.6 Schema versioning

JSON output is the stable contract for programmatic consumers (agents and scripts), not the human-readable text. The schema carries `schema_version`. Skills target a specific schema version. Breaking changes bump the major version; additive changes bump the minor and remain backward-compatible.

v0.1 ships `lazyadmin.snapshot.v1` and `lazyadmin.diff.v1`. The agent skill targets `v1`.

### 31.7 Why this matters

Tools in this space (lazydocker, port, isd) are human-only. A first-class agent integration is a real differentiator and reflects how local dev work is changing: agents are spawning processes, and tools that don't account for them produce orphans and corrupted state.

---

## 32. Open questions

```text
Should /proc/<pid>/fd/1 tail-file detection ship in v0.1 or v0.2?
  Recommended: v0.2. Useful but not blocking.

Should the manual cgroup fallback for `lazyadmin run` ship before v0.1 release?
  Recommended: no. systemd-run path covers ~95% of dev environments. Ship v0.1 without it; add in v0.2.

How aggressively should the agent skill auto-tag runs?
  Recommended: require explicit --tag, but offer a derivation hint when omitted.

Should `pause-restart` track its own pause registry separately from systemd's `mask`?
  Recommended: yes, so resume-restart works idempotently and doctor can list what's paused.

Should we infer HTTP-ish listeners safely enough to enable `o` open URL by default?
  Recommended: enable for localhost TCP listeners on common HTTP ports; require explicit
  config to enable for non-loopback.

Should Unix sockets show in the main UI by default?
  Recommended: hidden in Everything, visible in Ports view, always present in JSON export.

What's the policy for tracked runs that survive across reboots?
  $XDG_RUNTIME_DIR is wiped on reboot; runs do not survive. The registry rebuilds on startup
  with no entries, which is the correct behavior. Doctor can mention this.
```

---

## 33. Source references

These references informed the technical decisions in this spec:

- Ratatui crate documentation: https://docs.rs/ratatui/latest/ratatui/
- systemd D-Bus interface: https://man7.org/linux/man-pages/man5/org.freedesktop.systemd1.5.html
- systemd.socket man page: https://man7.org/linux/man-pages/man5/systemd.socket.5.html
- systemd-run man page: https://man7.org/linux/man-pages/man1/systemd-run.1.html
- prctl PR_SET_CHILD_SUBREAPER: https://man7.org/linux/man-pages/man2/prctl.2.html
- journalctl man page: https://man7.org/linux/man-pages/man1/journalctl.1.html
- Linux `/proc/net/tcp` kernel docs: https://www.kernel.org/doc/html/latest/networking/proc_net_tcp.html
- Linux `/proc/<pid>/fd` man page: https://man7.org/linux/man-pages/man5/proc_pid_fd.5.html
- Docker Engine API docs: https://docs.docker.com/reference/api/engine/
- Docker port publishing docs: https://docs.docker.com/engine/network/port-publishing/
- Docker Engine security docs: https://docs.docker.com/engine/security/
- Docker Compose services/labels docs: https://docs.docker.com/reference/compose-file/services/
- Podman REST API docs: https://docs.podman.io/en/latest/_static/api.html
- bollard crate: https://docs.rs/bollard/ (Podman as first-class runtime, schema 1.52)
- procfs crate: https://docs.rs/procfs/
- netlink-packet-sock-diag: https://docs.rs/netlink-packet-sock-diag/ (noted as stale; v0.2 candidate)
- cgroups v2 documentation: https://docs.kernel.org/admin-guide/cgroup-v2.html
- Apple launchd Terminal guide: https://support.apple.com/guide/terminal/script-management-with-launchd-apdc6c1077b-5d5d-4d35-9c19-60f2397b2369/mac
- `isd`: https://github.com/kainctl/isd
- `lazydocker`: https://github.com/jesseduffield/lazydocker
- `port`: https://github.com/enrell/port
