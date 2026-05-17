use crate::model::Protocol;
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selector {
    Socket(SocketSelector),
    Unix(PathBuf),
    Pid(i32),
    Unit(String),
    Container(String),
    Compose { project: String, service: String },
    Project(String),
    Run(String),
    Tag(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocketSelector {
    pub protocol: Protocol,
    pub host: Option<String>,
    pub port: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message} Hint: {hint}")]
pub struct SelectorError {
    pub message: String,
    pub hint: String,
}
impl SelectorError {
    fn new(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            hint: hint.into(),
        }
    }
}

pub fn parse_selector(input: &str) -> Result<Selector, SelectorError> {
    if let Some(path) = input.strip_prefix("unix://") {
        return Ok(Selector::Unix(PathBuf::from(path)));
    }
    for (prefix, ctor) in [
        ("pid:", "pid"),
        ("unit:", "unit"),
        ("container:", "container"),
        ("project:", "project"),
        ("run:", "run"),
        ("tag:", "tag"),
    ] {
        if let Some(rest) = input.strip_prefix(prefix) {
            return match ctor {
                "pid" => rest
                    .parse()
                    .map(Selector::Pid)
                    .map_err(|_| SelectorError::new("invalid PID selector", "use pid:42420")),
                "unit" => Ok(Selector::Unit(rest.into())),
                "container" => Ok(Selector::Container(rest.into())),
                "project" => Ok(Selector::Project(rest.into())),
                "run" => Ok(Selector::Run(rest.into())),
                "tag" => Ok(Selector::Tag(rest.into())),
                _ => unreachable!(),
            };
        }
    }
    if let Some(rest) = input.strip_prefix("compose:") {
        let (project, service) = rest.split_once('/').ok_or_else(|| {
            SelectorError::new("invalid compose selector", "use compose:project/service")
        })?;
        return Ok(Selector::Compose {
            project: project.into(),
            service: service.into(),
        });
    }
    parse_socket(input).map(Selector::Socket)
}
fn parse_socket(input: &str) -> Result<SocketSelector, SelectorError> {
    let (protocol, rest) = if let Some(r) = input.strip_prefix("tcp/") {
        (Protocol::Tcp, r)
    } else if let Some(r) = input.strip_prefix("udp/") {
        (Protocol::Udp, r)
    } else {
        (Protocol::Any, input)
    };
    if let Some(port) = rest.strip_prefix(':') {
        return Ok(SocketSelector {
            protocol,
            host: None,
            port: parse_port(port)?,
        });
    }
    if let Some(after) = rest.strip_prefix('[') {
        let (host, tail) = after
            .split_once(']')
            .ok_or_else(|| SelectorError::new("unclosed IPv6 literal", "use [::1]:3000"))?;
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| SelectorError::new("IPv6 selector missing port", "use [::1]:3000"))?;
        host.parse::<IpAddr>()
            .map_err(|_| SelectorError::new("invalid IPv6 address", "use [::1]:3000"))?;
        return Ok(SocketSelector {
            protocol,
            host: Some(host.into()),
            port: parse_port(port)?,
        });
    }
    if rest.matches(':').count() > 1 {
        return Err(SelectorError::new(
            "unbracketed IPv6 host/port selector",
            "wrap IPv6 literals in brackets, e.g. [::1]:3000",
        ));
    }
    let (host, port) = rest.rsplit_once(':').ok_or_else(|| {
        SelectorError::new(
            "unknown selector",
            "try :3000, tcp/[::1]:3000, pid:42420, unit:name.service",
        )
    })?;
    Ok(SocketSelector {
        protocol,
        host: Some(host.into()),
        port: parse_port(port)?,
    })
}
fn parse_port(s: &str) -> Result<u16, SelectorError> {
    s.parse::<u16>()
        .map_err(|_| SelectorError::new("invalid port", "ports must be 0-65535"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn examples() {
        for s in [
            ":3000",
            "127.0.0.1:3000",
            "[::1]:3000",
            "[::]:3000",
            "tcp/:3000",
            "tcp/127.0.0.1:3000",
            "tcp/[::1]:3000",
            "udp/[::]:5353",
            "unix:///tmp/app.sock",
            "pid:42420",
            "unit:dev-api.service",
            "unit:dev-api.socket",
            "container:localdb-postgres-1",
            "compose:localdb/postgres",
            "project:acme/web",
            "project:~/src/acme/web",
            "run:r-7f9a",
            "tag:acme-web",
        ] {
            parse_selector(s).unwrap_or_else(|e| panic!("{s}: {e}"));
        }
    }
    #[test]
    fn rejects() {
        assert!(parse_selector("::1:3000").is_err());
        assert!(parse_selector("[::1]").is_err());
    }
    #[test]
    fn protocol_default_any() {
        let Selector::Socket(s) = parse_selector(":3000").unwrap() else {
            panic!()
        };
        assert_eq!(s.protocol, Protocol::Any);
    }

    #[test]
    fn tcp_prefix_selects_tcp() {
        let Selector::Socket(s) = parse_selector("tcp/:3000").unwrap() else {
            panic!()
        };
        assert_eq!(s.protocol, Protocol::Tcp);
        assert_eq!(s.port, 3000);
        assert!(s.host.is_none());
    }

    #[test]
    fn udp_prefix_selects_udp_with_host() {
        let Selector::Socket(s) = parse_selector("udp/127.0.0.1:5353").unwrap() else {
            panic!()
        };
        assert_eq!(s.protocol, Protocol::Udp);
        assert_eq!(s.port, 5353);
        assert_eq!(s.host.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn pid_selector_carries_integer() {
        let Selector::Pid(pid) = parse_selector("pid:42420").unwrap() else {
            panic!()
        };
        assert_eq!(pid, 42420);
    }

    #[test]
    fn pid_selector_rejects_non_numeric() {
        let err = parse_selector("pid:abc").unwrap_err();
        assert!(err.message.contains("invalid PID"));
        assert!(err.hint.contains("pid:42420"));
    }

    #[test]
    fn port_above_u16_rejected() {
        let err = parse_selector(":70000").unwrap_err();
        assert!(err.message.contains("invalid port"));
    }

    #[test]
    fn unix_selector_carries_path() {
        let Selector::Unix(p) = parse_selector("unix:///tmp/app.sock").unwrap() else {
            panic!()
        };
        assert_eq!(p.to_string_lossy(), "/tmp/app.sock");
    }

    #[test]
    fn unit_run_tag_container_project_extract_rest() {
        match parse_selector("unit:dev.service").unwrap() {
            Selector::Unit(u) => assert_eq!(u, "dev.service"),
            _ => panic!("expected Unit"),
        }
        match parse_selector("run:r-1").unwrap() {
            Selector::Run(r) => assert_eq!(r, "r-1"),
            _ => panic!("expected Run"),
        }
        match parse_selector("tag:my-tag").unwrap() {
            Selector::Tag(t) => assert_eq!(t, "my-tag"),
            _ => panic!("expected Tag"),
        }
        match parse_selector("container:db-1").unwrap() {
            Selector::Container(c) => assert_eq!(c, "db-1"),
            _ => panic!("expected Container"),
        }
        match parse_selector("project:web").unwrap() {
            Selector::Project(p) => assert_eq!(p, "web"),
            _ => panic!("expected Project"),
        }
    }

    #[test]
    fn compose_selector_requires_slash() {
        // Valid case.
        let Selector::Compose { project, service } = parse_selector("compose:acme/web").unwrap()
        else {
            panic!("expected Compose")
        };
        assert_eq!(project, "acme");
        assert_eq!(service, "web");

        // Missing slash -> error with hint.
        let err = parse_selector("compose:acme-web").unwrap_err();
        assert!(err.message.contains("invalid compose"));
        assert!(err.hint.contains("compose:project/service"));
    }

    #[test]
    fn unbracketed_ipv6_with_host_returns_bracket_hint() {
        // "fe80::1:3000" has more than one colon and no leading `:`, no `[` —
        // selector should explicitly point the user at bracket notation.
        let err = parse_selector("fe80::1:3000").unwrap_err();
        let msg_lc = err.message.to_lowercase();
        assert!(
            msg_lc.contains("ipv6") || msg_lc.contains("unbracketed"),
            "unexpected message: {}",
            err.message
        );
        assert!(err.hint.to_lowercase().contains("bracket"));
    }

    #[test]
    fn unbracketed_ipv6_starting_with_colon_falls_through_to_port_parse() {
        // "::1:3000" lexically starts with `:`, so the parser strips the leading
        // colon and tries to parse ":1:3000" as a port. This is still an error,
        // just a different one — pin the current behaviour.
        let err = parse_selector("::1:3000").unwrap_err();
        assert!(err.message.contains("invalid port"));
    }

    #[test]
    fn ipv6_bracket_without_port_rejected() {
        let err = parse_selector("[::1]").unwrap_err();
        assert!(err.message.contains("missing port"));
    }

    #[test]
    fn ipv6_unclosed_bracket_rejected() {
        let err = parse_selector("[::1:3000").unwrap_err();
        assert!(err.message.contains("unclosed"));
    }

    #[test]
    fn ipv6_with_invalid_address_rejected() {
        let err = parse_selector("[zz::]:3000").unwrap_err();
        assert!(err.message.contains("invalid IPv6"));
    }

    #[test]
    fn selector_error_display_includes_message_and_hint() {
        let err = parse_selector(":notaport").unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("invalid port"));
        assert!(s.contains("Hint:"));
    }

    #[test]
    fn fully_garbled_input_returns_unknown_selector_hint() {
        let err = parse_selector("not-a-selector").unwrap_err();
        assert!(err.message.contains("unknown selector"));
    }

    #[test]
    fn selector_round_trips_through_json() {
        let s = parse_selector("tcp/[::1]:3000").unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: Selector = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
