use std::fmt;
const PATTERNS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "api_key",
    "authorization",
    "credential",
    "session",
    "cookie",
    "private_key",
];
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    PATTERNS.iter().any(|p| k == *p || k.contains(p))
}
pub fn redact_kv(key: &str, value: &str) -> String {
    if is_sensitive_key(key) {
        "<redacted>".into()
    } else {
        redact_url_userinfo(value)
    }
}
pub fn redact_cmdline(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            out.push("<redacted>".into());
            redact_next = false;
            continue;
        }
        if let Some((k, _)) = arg.split_once('=') {
            let key = k.trim_start_matches('-');
            if is_sensitive_key(key) {
                out.push(format!("{k}=<redacted>"));
                continue;
            }
        }
        let key = arg.trim_start_matches('-');
        if arg.starts_with('-') && is_sensitive_key(key) {
            out.push(arg.clone());
            redact_next = true;
        } else {
            out.push(redact_url_userinfo(arg));
        }
    }
    out
}
pub fn redact_env<I, K, V>(pairs: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    pairs
        .into_iter()
        .map(|(k, v)| (k.as_ref().into(), redact_kv(k.as_ref(), v.as_ref())))
        .collect()
}
pub fn redact_url_userinfo(value: &str) -> String {
    if let Some(scheme_end) = value.find("://") {
        let rest = &value[scheme_end + 3..];
        if let Some(at) = rest.find('@') {
            let userinfo = &rest[..at];
            if let Some(colon) = userinfo.find(':') {
                return format!(
                    "{}://{}:<redacted>@{}",
                    &value[..scheme_end],
                    &userinfo[..colon],
                    &rest[at + 1..]
                );
            }
        }
    }
    value.into()
}
#[derive(Clone, PartialEq, Eq)]
pub struct Redacted<T>(pub T);
impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reveal<T>(pub T);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn env_redaction() {
        assert_eq!(redact_kv("API_TOKEN", "abc"), "<redacted>");
        assert_eq!(redact_kv("PORT", "3000"), "3000");
    }
    #[test]
    fn cmdline_redaction() {
        let v = redact_cmdline(&[
            "--token".into(),
            "abc".into(),
            "--password=def".into(),
            "DATABASE_URL=postgres://u:p@h/db".into(),
        ]);
        assert_eq!(v[1], "<redacted>");
        assert_eq!(v[2], "--password=<redacted>");
        assert_eq!(v[3], "DATABASE_URL=postgres://u:<redacted>@h/db");
    }
    #[test]
    fn url_userinfo() {
        assert_eq!(
            redact_url_userinfo("postgres://user:pass@host/db"),
            "postgres://user:<redacted>@host/db"
        );
    }
    #[test]
    fn mixed_case() {
        assert!(is_sensitive_key("Private_Key"));
    }
    #[test]
    fn false_positive() {
        assert!(!is_sensitive_key("attempt"));
        assert!(!is_sensitive_key("port"));
    }
}
