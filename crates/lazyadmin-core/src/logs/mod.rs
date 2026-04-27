use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogOptions {
    pub tail: Option<usize>,
    pub follow: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub timestamp: Option<DateTime<Utc>>,
    pub source: String,
    pub stream: Option<String>,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogStream {
    pub source: String,
    pub lines: Vec<LogLine>,
    pub unavailable_message: Option<String>,
}

pub trait LogProvider {
    fn logs(&self, selector: &str, options: &LogOptions) -> anyhow::Result<LogStream>;
}

pub fn direct_unavailable(selector: &str) -> LogStream {
    LogStream { source: selector.into(), lines: vec![], unavailable_message: Some("No managed log source found. This process was started directly from a shell. To capture logs in the future, restart it under lazyadmin run -- <your command>.".into()) }
}
