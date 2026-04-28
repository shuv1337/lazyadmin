use crate::{config::Config, graph::Graph, model::*};
use chrono::Utc;
use futures::{Stream, stream::SelectAll};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct ViewSnapshot {
    pub snapshot: Snapshot,
    pub hidden_count: usize,
}

fn prov(claim: &str, e: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "correlate".into(),
        claim: claim.into(),
        evidence: e.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
fn warn(code: &str, msg: impl Into<String>, entity: Option<EntityRef>) -> Warning {
    Warning {
        severity: WarningSeverity::Warning,
        code: code.into(),
        message: msg.into(),
        entity,
        provenance: vec![prov("correlation warning", code, Confidence::Medium)],
    }
}

#[tracing::instrument(name = "correlate.run", skip_all)]
pub fn correlate(mut graph: Graph, _config: &Config) -> Graph {
    let start = Instant::now();
    classify_processes(&mut graph);
    correlate_portless(&mut graph);
    detect_conflicts(&mut graph);
    tracing::debug!(
        listeners = graph.listeners.len(),
        workloads = graph.workloads.len(),
        warnings = graph.warnings.len(),
        duration_ms = start.elapsed().as_millis(),
        "correlate complete"
    );
    graph
}

pub fn classify_cmdline(exe: Option<&str>, args: &[String]) -> Option<(RuntimeKind, &'static str)> {
    let joined = args.join(" ");
    let base = exe.unwrap_or_else(|| args.first().map(String::as_str).unwrap_or(""));
    if base.contains("kubectl") && args.iter().any(|a| a == "port-forward") {
        Some((RuntimeKind::KubectlPortForward, "TUNNEL"))
    } else if base.ends_with("ssh")
        && args.iter().any(|a| {
            a == "-L"
                || a == "-R"
                || a == "-D"
                || a.starts_with("-L")
                || a.starts_with("-R")
                || a.starts_with("-D")
        })
    {
        Some((RuntimeKind::SshTunnel, "TUNNEL"))
    } else if base.contains("socat") {
        Some((RuntimeKind::Socat, "TUNNEL"))
    } else if base.contains("cloudflared") {
        Some((RuntimeKind::Cloudflared, "TUNNEL"))
    } else if base.contains("ngrok")
        || joined.contains("minikube tunnel")
        || base.contains("telepresence")
    {
        Some((RuntimeKind::Direct, "TUNNEL"))
    } else if base.contains("caddy")
        || base.contains("traefik")
        || base.contains("envoy")
        || base.contains("linkerd-proxy")
        || base.contains("istio-proxy")
    {
        Some((RuntimeKind::Direct, "SIDECAR"))
    } else {
        None
    }
}

fn classify_processes(graph: &mut Graph) {
    let processes: Vec<_> = graph.processes.values().cloned().collect();
    for p in processes {
        let exe = p
            .exe
            .as_ref()
            .and_then(|x| x.file_name())
            .and_then(|s| s.to_str());
        if let Some((kind, badge)) = classify_cmdline(exe, &p.cmdline) {
            let wid = WorkloadId::new(format!("process:{}:{:?}", p.pid, kind));
            if !graph.workloads.contains_key(&wid) {
                graph.workloads.insert(
                    wid.clone(),
                    Workload {
                        id: wid.clone(),
                        display_name: p
                            .cmdline
                            .first()
                            .cloned()
                            .unwrap_or_else(|| exe.unwrap_or("process").into()),
                        runtime: kind,
                        state: WorkloadState::Running,
                        pids: vec![p.key.clone()],
                        listeners: vec![],
                        project: None,
                        manager: None,
                        source: Some(EntityRef::Process(p.key.clone())),
                        actions: vec![],
                        health: None,
                        metrics: None,
                        restart_policy: None,
                        lazyadmin_run_id: p.lazyadmin_run_id.clone(),
                        provenance: vec![prov(
                            "special process classifier",
                            format!("pid {}", p.pid),
                            Confidence::Medium,
                        )],
                    },
                );
            }
            graph.warnings.push(warn(
                badge,
                format!("special process detected: {}", p.cmdline.join(" ")),
                Some(EntityRef::Process(p.key.clone())),
            ));
        }
    }
}

fn correlate_portless(graph: &mut Graph) {
    let process_by_pid = processes_by_pid(graph);
    let children = children_index(graph);
    let listeners = graph.listeners.values().cloned().collect::<Vec<_>>();
    let mut updates = Vec::new();
    let mut edges = Vec::new();
    let mut warnings = Vec::new();

    for workload in graph
        .workloads
        .values()
        .filter(|workload| workload.runtime == RuntimeKind::Portless)
        .cloned()
    {
        let Some(route_pid) = portless_route_pid(&workload) else {
            continue;
        };
        let candidates = process_by_pid.get(&route_pid).cloned().unwrap_or_default();
        if candidates.len() != 1 {
            warnings.push(warn(
                "portless.route_pid_missing",
                format!(
                    "portless route {} references CLI pid {route_pid}, but procfs did not expose exactly one matching process",
                    workload.display_name
                ),
                Some(EntityRef::Workload(workload.id.clone())),
            ));
            continue;
        }
        let cli_key = candidates[0].clone();
        let descendants = descendant_processes(&cli_key, &children, 8);
        let mut listener_ids = Vec::new();
        let mut listener_owner_keys = Vec::new();
        for listener in &listeners {
            let matching_owners = listener
                .owners
                .iter()
                .filter_map(|owner| match owner {
                    EntityRef::Process(key) if descendants.contains(key) => Some(key.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if matching_owners.is_empty() {
                continue;
            }
            listener_ids.push(listener.id.clone());
            listener_owner_keys.extend(matching_owners);
            edges.push(Edge {
                kind: EdgeKind::WorkloadOwnsListener,
                from: EntityRef::Workload(workload.id.clone()),
                to: EntityRef::Listener(listener.id.clone()),
                provenance: vec![prov(
                    "procfs-descendant-of portless cli",
                    format!("procfs-descendant-of portless cli pid {route_pid}"),
                    Confidence::Medium,
                )],
            });
        }
        listener_owner_keys.sort_by(|a, b| {
            (a.pid, a.start_time_ticks, &a.boot_id).cmp(&(b.pid, b.start_time_ticks, &b.boot_id))
        });
        listener_owner_keys.dedup();
        listener_ids.sort();
        listener_ids.dedup();
        updates.push((workload.id, cli_key, listener_owner_keys, listener_ids));
    }

    for (workload_id, cli_key, pids, listeners) in updates {
        if let Some(workload) = graph.workloads.get_mut(&workload_id) {
            workload.source = Some(EntityRef::Process(cli_key));
            workload.pids = pids;
            workload.listeners = listeners;
        }
    }
    for edge in edges {
        if !graph.edges.iter().any(|existing| {
            existing.kind == edge.kind && existing.from == edge.from && existing.to == edge.to
        }) {
            graph.edges.push(edge);
        }
    }
    graph.warnings.extend(warnings);
}

fn processes_by_pid(graph: &Graph) -> HashMap<i32, Vec<ProcessKey>> {
    let mut out: HashMap<i32, Vec<ProcessKey>> = HashMap::new();
    for process in graph.processes.values() {
        out.entry(process.pid)
            .or_default()
            .push(process.key.clone());
    }
    out
}

fn children_index(graph: &Graph) -> HashMap<i32, Vec<ProcessKey>> {
    let mut out: HashMap<i32, Vec<ProcessKey>> = HashMap::new();
    for process in graph.processes.values() {
        if let Some(ppid) = process.ppid {
            out.entry(ppid).or_default().push(process.key.clone());
        }
    }
    out
}

fn descendant_processes(
    root: &ProcessKey,
    children: &HashMap<i32, Vec<ProcessKey>>,
    depth_cap: usize,
) -> HashSet<ProcessKey> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(root.pid, 0usize)]);
    while let Some((pid, depth)) = queue.pop_front() {
        if depth >= depth_cap {
            continue;
        }
        for child in children.get(&pid).into_iter().flatten() {
            if seen.insert(child.clone()) {
                queue.push_back((child.pid, depth + 1));
            }
        }
    }
    seen
}

fn portless_route_pid(workload: &Workload) -> Option<i32> {
    workload
        .provenance
        .iter()
        .find(|provenance| provenance.adapter == "portless")
        .and_then(|provenance| extract_prefixed_i32(&provenance.evidence, "pid="))
}

fn extract_prefixed_i32(text: &str, prefix: &str) -> Option<i32> {
    let start = text.find(prefix)? + prefix.len();
    let digits = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn detect_conflicts(graph: &mut Graph) {
    use std::collections::HashMap;
    let mut ports: HashMap<(String, Option<String>, Option<u16>), Vec<ListenerId>> = HashMap::new();
    for l in graph.listeners.values() {
        if l.exposure != Exposure::Loopback && l.port.is_some() {
            graph.warnings.push(warn(
                "PUBLIC",
                format!(
                    "public listener on {}:{}",
                    l.bind_addr.as_deref().unwrap_or("*"),
                    l.port.unwrap_or(0)
                ),
                Some(EntityRef::Listener(l.id.clone())),
            ));
        }
        if l.owners.len() > 1 {
            graph.warnings.push(warn(
                "CONFLICT",
                "multiple owners for listener",
                Some(EntityRef::Listener(l.id.clone())),
            ));
        }
        ports
            .entry((format!("{:?}", l.protocol), l.bind_addr.clone(), l.port))
            .or_default()
            .push(l.id.clone());
    }
    for ids in ports.values().filter(|v| v.len() > 1) {
        for id in ids {
            graph.warnings.push(warn(
                "CONFLICT",
                "same protocol/address/port appears multiple times (reuseport or namespace)",
                Some(EntityRef::Listener(id.clone())),
            ));
        }
    }
    let mut numeric: HashMap<u16, Vec<Protocol>> = HashMap::new();
    for l in graph.listeners.values() {
        if let Some(p) = l.port {
            numeric.entry(p).or_default().push(l.protocol.clone())
        }
    }
    for (p, protos) in numeric {
        if protos.contains(&Protocol::Tcp) && protos.contains(&Protocol::Udp) {
            graph.warnings.push(warn(
                "CONFLICT",
                format!("TCP and UDP both use port {p}"),
                None,
            ));
        }
    }
}

pub fn everything_filter(snapshot: &Snapshot, _config: &Config) -> ViewSnapshot {
    let mut out = snapshot.clone();
    let before = out.workloads.len();
    out.workloads
        .retain(|w| w.runtime != RuntimeKind::SystemdSystem);
    let hidden = before - out.workloads.len();
    ViewSnapshot {
        snapshot: out,
        hidden_count: hidden,
    }
}

#[derive(Clone, Debug)]
pub struct EventNormalizer {
    debounce_window: Duration,
    last_seen: HashMap<String, Instant>,
}

impl EventNormalizer {
    pub fn new(debounce_window: Duration) -> Self {
        Self {
            debounce_window,
            last_seen: HashMap::new(),
        }
    }

    pub fn normalize(&mut self, event: DiscoveryEvent) -> Option<DiscoveryEvent> {
        let key = serde_json::to_string(&(
            &event.kind,
            &event.entity,
            &event.changes,
            &event.adapter,
            &event.reason,
        ))
        .unwrap_or_else(|_| format!("{:?}", event.kind));
        let now = Instant::now();
        if self
            .last_seen
            .get(&key)
            .is_some_and(|previous| now.duration_since(*previous) < self.debounce_window)
        {
            return None;
        }
        self.last_seen.insert(key, now);
        Some(event)
    }
}

#[derive(Clone, Debug, Default)]
pub struct EventDropCounter {
    dropped: Arc<AtomicU64>,
}

impl EventDropCounter {
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn increment(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct EventFanIn {
    streams: SelectAll<futures::stream::BoxStream<'static, DiscoveryEvent>>,
    buffer: VecDeque<DiscoveryEvent>,
    capacity: usize,
    drop_counter: EventDropCounter,
    normalizer: EventNormalizer,
}

impl std::fmt::Debug for EventFanIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFanIn")
            .field("stream_count", &self.streams.len())
            .field("buffer_len", &self.buffer.len())
            .field("capacity", &self.capacity)
            .field("dropped", &self.drop_counter.dropped())
            .finish()
    }
}

impl EventFanIn {
    pub fn new(
        streams: Vec<futures::stream::BoxStream<'static, DiscoveryEvent>>,
        capacity: usize,
        debounce_window: Duration,
    ) -> (Self, EventDropCounter) {
        let counter = EventDropCounter::default();
        let mut select = SelectAll::new();
        for stream in streams {
            select.push(stream);
        }
        (
            Self {
                streams: select,
                buffer: VecDeque::new(),
                capacity: capacity.max(1),
                drop_counter: counter.clone(),
                normalizer: EventNormalizer::new(debounce_window),
            },
            counter,
        )
    }

    pub fn push_event_for_test(&mut self, event: DiscoveryEvent) {
        self.push_event(event);
    }

    fn push_event(&mut self, event: DiscoveryEvent) {
        let Some(event) = self.normalizer.normalize(event) else {
            return;
        };
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
            self.drop_counter.increment();
        }
        self.buffer.push_back(event);
    }
}

impl Stream for EventFanIn {
    type Item = DiscoveryEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(Some(event)) = Pin::new(&mut self.streams).poll_next(cx) {
            self.push_event(event);
        }
        if let Some(event) = self.buffer.pop_front() {
            Poll::Ready(Some(event))
        } else if self.streams.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }
}

#[tracing::instrument(name = "graph.correlate", skip_all, fields(result = "ok"))]
pub fn correlate_placeholder() {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifiers() {
        let cases = [
            (
                "kubectl",
                vec!["kubectl", "port-forward", "svc/x", "8080:80"],
            ),
            ("ssh", vec!["ssh", "-L", "1:2:3", "h"]),
            ("socat", vec!["socat"]),
            ("cloudflared", vec!["cloudflared"]),
            ("ngrok", vec!["ngrok"]),
            ("caddy", vec!["caddy"]),
            ("traefik", vec!["traefik"]),
            ("minikube", vec!["minikube", "tunnel"]),
            ("telepresence", vec!["telepresence"]),
            ("envoy", vec!["envoy"]),
            ("linkerd-proxy", vec!["linkerd-proxy"]),
            ("istio-proxy", vec!["istio-proxy"]),
        ];
        for (exe, args) in cases {
            let a = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(classify_cmdline(Some(exe), &a).is_some(), "{exe}");
        }
    }
    #[test]
    fn two_tier_filter() {
        let mut s = Snapshot::empty();
        s.workloads.push(Workload {
            id: WorkloadId::new("w"),
            display_name: "w".into(),
            runtime: RuntimeKind::SystemdSystem,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![],
            project: None,
            manager: None,
            source: None,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        let v = everything_filter(&s, &Config::default());
        assert_eq!(v.hidden_count, 1);
        assert!(v.snapshot.workloads.is_empty());
    }

    #[test]
    fn discovery_event_normalizer_dedupes_storms() {
        let mut n = EventNormalizer::new(Duration::from_secs(60));
        let event = DiscoveryEvent::heartbeat("procfs");
        assert!(n.normalize(event.clone()).is_some());
        assert!(n.normalize(event).is_none());
    }

    #[test]
    fn fan_in_drops_oldest_and_counts_overflow() {
        let (mut fan_in, drops) = EventFanIn::new(vec![], 2, Duration::from_millis(0));
        fan_in.push_event_for_test(DiscoveryEvent::heartbeat("a"));
        fan_in.push_event_for_test(DiscoveryEvent::heartbeat("b"));
        fan_in.push_event_for_test(DiscoveryEvent::heartbeat("c"));
        assert_eq!(drops.dropped(), 1);
        assert_eq!(
            fan_in.buffer.pop_front().unwrap().adapter.as_deref(),
            Some("b")
        );
        assert_eq!(
            fan_in.buffer.pop_front().unwrap().adapter.as_deref(),
            Some("c")
        );
    }

    #[test]
    fn portless_correlation_resolves_cli_and_descendant_listener() {
        let cli = ProcessKey {
            pid: 100,
            boot_id: "boot".into(),
            start_time_ticks: 10,
        };
        let child = ProcessKey {
            pid: 101,
            boot_id: "boot".into(),
            start_time_ticks: 11,
        };
        let listener_id = ListenerId::new("tcp:127.0.0.1:3737:99");
        let workload_id = WorkloadId::new("portless:demo");
        let mut graph = Graph::default();
        graph
            .processes
            .insert(cli.clone(), process(cli.clone(), None));
        graph
            .processes
            .insert(child.clone(), process(child.clone(), Some(100)));
        graph.listeners.insert(
            listener_id.clone(),
            Listener {
                id: listener_id.clone(),
                protocol: Protocol::Tcp,
                family: AddressFamily::Ipv4,
                bind_addr: Some("127.0.0.1".into()),
                port: Some(3737),
                path: None,
                state: ListenerState::Listen,
                netns: "host".into(),
                socket_inode: Some(99),
                exposure: Exposure::Loopback,
                owners: vec![EntityRef::Process(child.clone())],
                confidence: Confidence::High,
                provenance: vec![],
                first_seen: Utc::now(),
                last_seen: Utc::now(),
                dual_stack_state: DualStackState::NotApplicable,
            },
        );
        graph.workloads.insert(
            workload_id.clone(),
            Workload {
                id: workload_id.clone(),
                display_name: "demo".into(),
                runtime: RuntimeKind::Portless,
                state: WorkloadState::Running,
                pids: vec![],
                listeners: vec![],
                project: None,
                manager: None,
                source: None,
                actions: vec![],
                health: None,
                metrics: None,
                restart_policy: None,
                lazyadmin_run_id: None,
                provenance: vec![Provenance {
                    adapter: "portless".into(),
                    claim: "route cli pid".into(),
                    evidence: "routes.json pid=100 state_dir=/tmp/example hostname=demo port=3737"
                        .into(),
                    confidence: Confidence::High,
                    timestamp: Utc::now(),
                }],
            },
        );

        let graph = correlate(graph, &Config::default());
        let workload = graph.workloads.get(&workload_id).unwrap();
        assert_eq!(workload.source, Some(EntityRef::Process(cli)));
        assert_eq!(workload.pids, vec![child]);
        assert_eq!(workload.listeners, vec![listener_id.clone()]);
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::WorkloadOwnsListener
                && edge.from == EntityRef::Workload(workload_id.clone())
                && edge.to == EntityRef::Listener(listener_id.clone())
        }));
    }

    fn process(key: ProcessKey, ppid: Option<i32>) -> Process {
        Process {
            pid: key.pid,
            start_time_ticks: key.start_time_ticks,
            boot_id: key.boot_id.clone(),
            key,
            user: None,
            exe: None,
            cmdline: vec![],
            cwd: None,
            ppid,
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: None,
            lazyadmin_run_id: None,
            environment: RedactedEnvironmentSummary::default(),
            provenance: vec![],
        }
    }
}
