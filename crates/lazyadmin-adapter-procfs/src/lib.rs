#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

use async_trait::async_trait;
use chrono::Utc;
use lazyadmin_core::{
    config::Config,
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
    redact::redact_cmdline,
};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::{Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug)]
pub struct ProcfsAdapter {
    pub config: Config,
    root: PathBuf,
}
#[derive(Clone, Debug)]
pub struct RawProcListener {
    pub protocol: Protocol,
    pub family: AddressFamily,
    pub addr: Option<String>,
    pub port: Option<u16>,
    pub path: Option<PathBuf>,
    pub state_hex: String,
    pub inode: SocketInode,
    pub netns: NamespaceId,
}
#[derive(Clone, Debug)]
pub struct RawProcProcess {
    pub process: Process,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SocketInode(pub u64);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NamespaceId(pub String);
#[derive(Clone, Debug, Default)]
pub struct ProcScanCache {
    pub processes: HashMap<ProcessKey, Process>,
}

impl ProcfsAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            root: PathBuf::from("/proc"),
        }
    }
    pub fn with_root(config: Config, root: impl Into<PathBuf>) -> Self {
        Self {
            config,
            root: root.into(),
        }
    }
}

fn prov(claim: &str, evidence: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "procfs".into(),
        claim: claim.into(),
        evidence: evidence.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
fn warn(code: &str, message: impl Into<String>, entity: Option<EntityRef>) -> Warning {
    Warning {
        severity: WarningSeverity::Warning,
        code: code.into(),
        message: message.into(),
        entity,
        provenance: vec![prov("warning", code, Confidence::Medium)],
    }
}

#[async_trait]
impl DiscoveryAdapter for ProcfsAdapter {
    fn name(&self) -> &'static str {
        "procfs"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }
    async fn health(&self) -> AdapterHealth {
        AdapterHealth {
            adapter: "procfs".into(),
            available: self.root.exists(),
            message: Some(format!(
                "proc readable={} proc_net readable={} ss_available={}",
                self.root.exists(),
                self.root.join("net").exists(),
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg("command -v ss >/dev/null 2>&1")
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            )),
        }
    }
    #[tracing::instrument(name = "adapter.procfs.discover", skip_all)]
    async fn discover(&self, _ctx: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let start = Instant::now();
        let mut out = DiscoveryOutput::default();
        let mut raw = Vec::new();
        let mut warnings = Vec::new();
        for (file, proto, fam) in [
            ("tcp", Protocol::Tcp, AddressFamily::Ipv4),
            ("tcp6", Protocol::Tcp, AddressFamily::Ipv6),
            ("udp", Protocol::Udp, AddressFamily::Ipv4),
            ("udp6", Protocol::Udp, AddressFamily::Ipv6),
        ] {
            match parse_inet_file(&self.root.join("net").join(file), proto, fam) {
                Ok(mut v) => raw.append(&mut v),
                Err(e) => warnings.push(warn(
                    "proc_net_parse",
                    format!("failed to parse {file}: {e}"),
                    None,
                )),
            }
        }
        if let Ok(mut v) = parse_unix_file(&self.root.join("net/unix")) {
            raw.append(&mut v);
        }
        let inodes: HashSet<u64> = raw.iter().map(|l| l.inode.0).filter(|i| *i != 0).collect();
        let boot_id = fs::read_to_string(self.root.join("sys/kernel/random/boot_id"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let mut processes = scan_processes(&self.root, &boot_id, &mut warnings);
        let owners = map_socket_owners(&self.root, &processes, &inodes, &mut warnings);
        let now = Utc::now();
        for r in raw {
            let id = ListenerId::new(format!(
                "{}:{}:{}:{}",
                proto_s(&r.protocol),
                r.addr.clone().unwrap_or_default(),
                r.port.unwrap_or(0),
                r.inode.0
            ));
            let ent_owners: Vec<_> = owners
                .get(&r.inode.0)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(EntityRef::Process)
                .collect();
            let mut provenance = vec![prov(
                "parsed listener",
                format!("/proc/net inode {}", r.inode.0),
                Confidence::High,
            )];
            let exposure = exposure(r.addr.as_deref(), &r.family, &r.protocol);
            let listener = Listener {
                id: id.clone(),
                protocol: r.protocol.clone(),
                family: r.family,
                bind_addr: r.addr.clone(),
                port: r.port,
                path: r.path,
                state: if r.protocol == Protocol::Udp {
                    ListenerState::Bound
                } else {
                    ListenerState::Listen
                },
                netns: r.netns.0,
                socket_inode: Some(r.inode.0),
                exposure,
                owners: ent_owners.clone(),
                confidence: Confidence::High,
                provenance: {
                    provenance.shrink_to_fit();
                    provenance
                },
                first_seen: now,
                last_seen: now,
            };
            if matches!(listener.bind_addr.as_deref(), Some("0.0.0.0") | Some("::")) {
                warnings.push(warn(
                    "wide_bind",
                    "reachable beyond localhost depending on firewall/routing",
                    Some(EntityRef::Listener(id.clone())),
                ));
            }
            if listener.bind_addr.as_deref() == Some("::") {
                warnings.push(warn("possible_dual_stack", "IPv6 wildcard may also accept IPv4 unless IPV6_V6ONLY is set; /proc/net cannot prove this", Some(EntityRef::Listener(id.clone()))));
            }
            out.listeners.push(listener);
            for owner in ent_owners {
                out.edges.push(Edge {
                    kind: EdgeKind::ProcessOwnsListener,
                    from: owner,
                    to: EntityRef::Listener(id.clone()),
                    provenance: vec![prov(
                        "socket inode owner",
                        format!("inode {}", r.inode.0),
                        Confidence::High,
                    )],
                });
            }
        }
        for p in processes.drain(..) {
            out.processes.push(p);
        }
        out.warnings = warnings;
        tracing::info!(
            listener_count = out.listeners.len(),
            process_count = out.processes.len(),
            duration_ms = start.elapsed().as_millis(),
            "procfs discovery complete"
        );
        Ok(out)
    }
}
fn proto_s(p: &Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
        Protocol::Unix => "unix",
        Protocol::Any => "any",
    }
}

pub fn parse_inet_file(
    path: &Path,
    protocol: Protocol,
    family: AddressFamily,
) -> io::Result<Vec<RawProcListener>> {
    let text = fs::read_to_string(path)?;
    Ok(parse_inet(&text, protocol, family))
}
pub fn parse_inet(text: &str, protocol: Protocol, family: AddressFamily) -> Vec<RawProcListener> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<_> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        let st = cols[3];
        if protocol == Protocol::Tcp && st != "0A" {
            continue;
        }
        let Some((addr_hex, port_hex)) = cols[1].split_once(':') else {
            continue;
        };
        let port = u16::from_str_radix(port_hex, 16).ok();
        let addr = decode_addr(addr_hex, &family);
        let inode = cols[9].parse().unwrap_or(0);
        out.push(RawProcListener {
            protocol: protocol.clone(),
            family: family.clone(),
            addr,
            port,
            path: None,
            state_hex: st.into(),
            inode: SocketInode(inode),
            netns: NamespaceId("host".into()),
        });
    }
    out
}
fn decode_addr(hex: &str, fam: &AddressFamily) -> Option<String> {
    match fam {
        AddressFamily::Ipv4 if hex.len() == 8 => {
            let b = u32::from_str_radix(hex, 16).ok()?.to_le_bytes();
            Some(Ipv4Addr::from(b).to_string())
        }
        AddressFamily::Ipv6 if hex.len() == 32 => {
            let mut bytes = [0u8; 16];
            for i in 0..16 {
                bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            for chunk in bytes.chunks_exact_mut(4) {
                chunk.reverse();
            }
            Some(Ipv6Addr::from(bytes).to_string())
        }
        _ => None,
    }
}
pub fn parse_unix_file(path: &Path) -> io::Result<Vec<RawProcListener>> {
    let text = fs::read_to_string(path)?;
    Ok(parse_unix(&text))
}
pub fn parse_unix(text: &str) -> Vec<RawProcListener> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<_> = line.split_whitespace().collect();
        if cols.len() < 7 {
            continue;
        }
        let inode = cols[6].parse().unwrap_or(0);
        let p = cols.get(7).map(PathBuf::from);
        out.push(RawProcListener {
            protocol: Protocol::Unix,
            family: AddressFamily::Unix,
            addr: None,
            port: None,
            path: p,
            state_hex: cols[5].into(),
            inode: SocketInode(inode),
            netns: NamespaceId("host".into()),
        });
    }
    out
}
fn exposure(addr: Option<&str>, fam: &AddressFamily, proto: &Protocol) -> Exposure {
    if *proto == Protocol::Unix || *fam == AddressFamily::Unix {
        return Exposure::UnixLocal;
    }
    match addr {
        Some("127.0.0.1") | Some("::1") => Exposure::Loopback,
        Some("0.0.0.0") | Some("::") => Exposure::LanOrPublic,
        Some(a) if a.starts_with("127.") => Exposure::Loopback,
        Some(a) if a.starts_with("10.") || a.starts_with("192.168.") || a.starts_with("172.") => {
            Exposure::LanOrPublic
        }
        Some(_) => Exposure::Public,
        None => Exposure::Unknown,
    }
}

fn scan_processes(root: &Path, boot_id: &str, warnings: &mut Vec<Warning>) -> Vec<Process> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        match read_process(root, pid, boot_id) {
            Ok(p) => out.push(p),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => warnings.push(warn(
                "permission_denied",
                format!("permission denied reading pid {pid}"),
                None,
            )),
            Err(_) => {}
        }
    }
    out
}
fn read_process(root: &Path, pid: i32, boot_id: &str) -> io::Result<Process> {
    let dir = root.join(pid.to_string());
    let stat = fs::read_to_string(dir.join("stat"))?;
    let after = stat.rsplit_once(')').map(|(_, r)| r.trim()).unwrap_or("");
    let f: Vec<_> = after.split_whitespace().collect();
    let ppid = f.get(1).and_then(|s| s.parse().ok());
    let pgid = f.get(2).and_then(|s| s.parse().ok());
    let sid = f.get(3).and_then(|s| s.parse().ok());
    let start = f.get(19).and_then(|s| s.parse().ok()).unwrap_or(0);
    let cmdline = fs::read(dir.join("cmdline"))
        .map(|b| {
            b.split(|c| *c == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let key = ProcessKey {
        pid,
        boot_id: boot_id.into(),
        start_time_ticks: start,
    };
    let netns = fs::read_link(dir.join("ns/net"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    Ok(Process {
        key: key.clone(),
        pid,
        start_time_ticks: start,
        boot_id: boot_id.into(),
        user: uid_from_status(&dir.join("status")),
        exe: fs::read_link(dir.join("exe")).ok(),
        cmdline: redact_cmdline(&cmdline),
        cwd: fs::read_link(dir.join("cwd")).ok(),
        ppid,
        pgid,
        sid,
        cgroup: fs::read_to_string(dir.join("cgroup")).ok(),
        netns,
        container_id: None,
        systemd_unit: None,
        lazyadmin_run_id: None,
        environment: RedactedEnvironmentSummary::default(),
        provenance: vec![prov(
            "process scan",
            format!("/proc/{pid}"),
            Confidence::High,
        )],
    })
}
fn uid_from_status(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()?.lines().find_map(|l| {
        l.strip_prefix("Uid:")
            .and_then(|r| r.split_whitespace().next())
            .map(str::to_string)
    })
}
fn map_socket_owners(
    root: &Path,
    processes: &[Process],
    inodes: &HashSet<u64>,
    warnings: &mut Vec<Warning>,
) -> HashMap<u64, Vec<ProcessKey>> {
    let mut map: HashMap<u64, Vec<ProcessKey>> = HashMap::new();
    for p in processes {
        let fd = root.join(p.pid.to_string()).join("fd");
        let Ok(rd) = fs::read_dir(&fd) else { continue };
        for e in rd.flatten() {
            match fs::read_link(e.path()) {
                Ok(t) => {
                    if let Some(i) = parse_socket_target(&t.to_string_lossy()) {
                        if inodes.contains(&i) {
                            map.entry(i).or_default().push(p.key.clone())
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => warnings.push(warn(
                    "fd_permission_denied",
                    format!(
                        "permission denied reading fd for pid {} uid {:?}",
                        p.pid, p.user
                    ),
                    Some(EntityRef::Process(p.key.clone())),
                )),
                Err(_) => {}
            }
        }
    }
    map
}
pub fn parse_socket_target(s: &str) -> Option<u64> {
    s.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_net_ipv4() {
        let t = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:0BB8 00000000:0000 0A 0 0 0 0 0 123\n";
        let v = parse_inet(t, Protocol::Tcp, AddressFamily::Ipv4);
        assert_eq!(v[0].addr.as_deref(), Some("127.0.0.1"));
        assert_eq!(v[0].port, Some(3000));
    }
    #[test]
    fn proc_net_udp() {
        let t = "h\n0: 00000000:14E9 0:0 07 0 0 0 0 0 55\n";
        assert_eq!(parse_inet(t, Protocol::Udp, AddressFamily::Ipv4).len(), 1);
    }
    #[test]
    fn unix_socket() {
        let t = "Num RefCount Protocol Flags Type St Inode Path\n0 0 0 0 0001 01 42 /tmp/a.sock\n";
        assert_eq!(
            parse_unix(t)[0].path.as_ref().unwrap(),
            Path::new("/tmp/a.sock")
        );
    }
    #[test]
    fn socket_owner() {
        assert_eq!(parse_socket_target("socket:[123]"), Some(123));
    }
    #[test]
    fn exposure_class() {
        assert_eq!(
            exposure(Some("127.1.2.3"), &AddressFamily::Ipv4, &Protocol::Tcp),
            Exposure::Loopback
        );
    }
}
