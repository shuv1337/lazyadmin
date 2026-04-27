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
}
