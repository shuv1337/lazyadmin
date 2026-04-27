use crate::{config::Config, graph::Graph, model::*};
use chrono::Utc;
use std::time::Instant;

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
}
