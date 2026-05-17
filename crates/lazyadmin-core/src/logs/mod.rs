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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_unavailable_sets_source_and_explains() {
        let stream = direct_unavailable("pid:42");
        assert_eq!(stream.source, "pid:42");
        assert!(stream.lines.is_empty());
        let msg = stream.unavailable_message.expect("explanation");
        assert!(msg.contains("lazyadmin run"));
        assert!(msg.contains("directly from a shell"));
    }

    #[test]
    fn log_options_serialize_with_canonical_field_names() {
        let opts = LogOptions {
            tail: Some(50),
            follow: true,
        };
        let json = serde_json::to_string(&opts).unwrap();
        assert!(json.contains("\"tail\":50"));
        assert!(json.contains("\"follow\":true"));
        let back: LogOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(back, opts);
    }

    #[test]
    fn log_line_round_trips_through_json() {
        let line = LogLine {
            timestamp: None,
            source: "unit:dev-api.service".into(),
            stream: Some("stdout".into()),
            message: "hello".into(),
        };
        let json = serde_json::to_string(&line).unwrap();
        let back: LogLine = serde_json::from_str(&json).unwrap();
        assert_eq!(back, line);
    }

    #[test]
    fn log_stream_serializes_with_optional_message_present() {
        let stream = LogStream {
            source: "x".into(),
            lines: vec![LogLine {
                timestamp: None,
                source: "x".into(),
                stream: None,
                message: "line".into(),
            }],
            unavailable_message: None,
        };
        let json = serde_json::to_string(&stream).unwrap();
        // None unavailable_message still serializes (it is not skip_serializing).
        assert!(json.contains("unavailable_message"));
        let back: LogStream = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stream);
    }

    #[test]
    fn log_provider_can_be_implemented_by_user_type() {
        struct Fake;
        impl LogProvider for Fake {
            fn logs(&self, _selector: &str, _options: &LogOptions) -> anyhow::Result<LogStream> {
                Ok(direct_unavailable("-"))
            }
        }
        let result = Fake
            .logs(
                "-",
                &LogOptions {
                    tail: None,
                    follow: false,
                },
            )
            .unwrap();
        assert!(result.unavailable_message.is_some());
    }
}
