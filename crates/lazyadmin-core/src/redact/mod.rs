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

    #[test]
    fn substring_matches_sensitive_key_in_compound_names() {
        // "token" is in PATTERNS — any key that contains it should match.
        assert!(is_sensitive_key("GITHUB_TOKEN"));
        assert!(is_sensitive_key("slack_session_cookie"));
        assert!(is_sensitive_key("my_API_KEY"));
    }

    #[test]
    fn empty_string_key_is_not_sensitive() {
        assert!(!is_sensitive_key(""));
    }

    #[test]
    fn redact_kv_passes_through_non_sensitive() {
        // Non-sensitive key + value with no scheme:// userinfo -> identity.
        assert_eq!(redact_kv("HOST", "localhost"), "localhost");
        assert_eq!(redact_kv("COUNT", "42"), "42");
    }

    #[test]
    fn redact_kv_redacts_url_userinfo_even_for_safe_keys() {
        // DATABASE_URL is not a sensitive *key* but its value carries credentials.
        let result = redact_kv("DATABASE_URL", "postgres://u:p@h:5432/db");
        assert_eq!(result, "postgres://u:<redacted>@h:5432/db");
    }

    #[test]
    fn redact_url_userinfo_handles_url_without_password() {
        // Only username, no password — should not change because there is no colon.
        let raw = "https://user@github.com/repo";
        assert_eq!(redact_url_userinfo(raw), raw);
    }

    #[test]
    fn redact_url_userinfo_returns_input_when_no_at_sign() {
        assert_eq!(
            redact_url_userinfo("https://example.com/path"),
            "https://example.com/path"
        );
    }

    #[test]
    fn redact_url_userinfo_returns_input_when_no_scheme() {
        // No "://" — not a URL we recognise.
        assert_eq!(redact_url_userinfo("not a url"), "not a url");
    }

    #[test]
    fn redact_cmdline_handles_empty_input() {
        assert!(redact_cmdline(&[]).is_empty());
    }

    #[test]
    fn redact_cmdline_passes_through_plain_args() {
        let v = redact_cmdline(&[
            "server".into(),
            "--port".into(),
            "8080".into(),
            "--verbose".into(),
        ]);
        assert_eq!(
            v,
            vec![
                "server".to_string(),
                "--port".to_string(),
                "8080".to_string(),
                "--verbose".to_string()
            ]
        );
    }

    #[test]
    fn redact_cmdline_redacts_short_and_long_secret_flags() {
        let v = redact_cmdline(&["--secret".into(), "hunter2".into(), "--apikey=abc".into()]);
        assert_eq!(v[0], "--secret");
        assert_eq!(v[1], "<redacted>");
        assert_eq!(v[2], "--apikey=<redacted>");
    }

    #[test]
    fn redact_cmdline_redacts_url_userinfo_in_positional_args() {
        let v = redact_cmdline(&["db".into(), "postgres://u:p@h/db".into()]);
        assert_eq!(v[1], "postgres://u:<redacted>@h/db");
    }

    #[test]
    fn redact_env_returns_pairs_with_values_redacted() {
        let pairs = redact_env([
            ("PORT", "3000"),
            ("API_TOKEN", "abc"),
            ("DATABASE_URL", "postgres://u:p@h/db"),
        ]);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], ("PORT".to_string(), "3000".to_string()));
        assert_eq!(pairs[1].1, "<redacted>");
        assert_eq!(pairs[2].1, "postgres://u:<redacted>@h/db");
    }

    #[test]
    fn redacted_wrapper_obscures_debug_and_display() {
        let r = Redacted("my-secret".to_string());
        assert_eq!(format!("{:?}", r), "<redacted>");
        assert_eq!(format!("{}", r), "<redacted>");
    }

    #[test]
    fn reveal_wrapper_keeps_inner_visible_for_debug() {
        let r = Reveal("plain".to_string());
        let dbg = format!("{:?}", r);
        assert!(
            dbg.contains("plain"),
            "Reveal Debug should not redact: {dbg}"
        );
    }
}
