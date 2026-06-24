//! End-to-end smoke tests for the compiled `lazyadmin` binary.
//!
//! These exercise the CLI verbs documented in AGENTS.md as validation
//! commands. They run the binary directly and assert exit status + JSON
//! shape, locking in the public contract that scripts and agents call.
//!
//! No external dependencies (assert_cmd is intentionally avoided to keep
//! the dependency footprint small) — we shell out via `std::process::Command`
//! and look up the binary via `env!("CARGO_BIN_EXE_lazyadmin")`.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lazyadmin"))
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn lazyadmin binary");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn parse_json(stdout: &str) -> serde_json::Value {
    serde_json::from_str(stdout).unwrap_or_else(|e| {
        panic!("expected valid JSON on stdout, got error {e}:\n---\n{stdout}\n---")
    })
}

#[test]
fn help_subcommand_exits_zero() {
    let (ok, stdout, _) = run(&["--help"]);
    assert!(ok, "lazyadmin --help should succeed");
    assert!(stdout.contains("Local runtime control plane"));
}

#[test]
fn version_flag_prints_workspace_version() {
    let (ok, stdout, _) = run(&["--version"]);
    assert!(ok);
    assert!(
        stdout.contains("lazyadmin"),
        "expected program name in version output: {stdout}"
    );
}

#[test]
fn export_json_emits_snapshot_schema_v1() {
    let (ok, stdout, _) = run(&["export", "--json"]);
    assert!(ok, "lazyadmin export --json should succeed");
    let json = parse_json(&stdout);
    assert_eq!(
        json["schema_version"], "lazyadmin.snapshot.v1",
        "schema_version locked"
    );
    assert!(json["listeners"].is_array());
    assert!(json["workloads"].is_array());
    assert!(json["managers"].is_array());
    assert!(json["warnings"].is_array());
}

#[test]
fn config_check_json_returns_resolved_config() {
    let (ok, stdout, _) = run(&["config", "check", "--json"]);
    assert!(ok);
    let json = parse_json(&stdout);
    // The shape exposes at least adapters config and ui config sections.
    assert!(json.is_object(), "config check --json must be an object");
}

#[test]
fn doctor_json_returns_doctor_schema_v1() {
    let (ok, stdout, _) = run(&["doctor", "--json"]);
    assert!(ok, "doctor --json should succeed");
    let json = parse_json(&stdout);
    assert_eq!(json["schema_version"], "lazyadmin.doctor.v1");
    assert!(json["checks"].is_array());
}

#[test]
fn ps_json_returns_array() {
    let (ok, stdout, _) = run(&["ps", "--json"]);
    assert!(ok);
    let json = parse_json(&stdout);
    // ps can return either an array or an object with rows; just confirm valid.
    assert!(json.is_array() || json.is_object());
}

#[test]
fn public_json_returns_well_formed_value() {
    let (ok, stdout, _) = run(&["public", "--json"]);
    assert!(ok);
    let _ = parse_json(&stdout);
}

#[test]
fn conflicts_json_returns_well_formed_value() {
    let (ok, stdout, _) = run(&["conflicts", "--json"]);
    assert!(ok);
    let _ = parse_json(&stdout);
}

#[test]
fn projects_json_returns_well_formed_value() {
    let (ok, stdout, _) = run(&["projects", "--json"]);
    assert!(ok);
    let _ = parse_json(&stdout);
}

#[test]
fn overview_json_returns_digest_shape() {
    let (ok, stdout, _) = run(&["overview", "--json"]);
    assert!(ok);
    let json = parse_json(&stdout);
    // Digest sections. The projects section is named `your_projects`.
    for key in ["exposed", "conflicts", "triage", "your_projects"] {
        assert!(
            json.get(key).is_some(),
            "overview digest missing key `{key}`:\n{json}"
        );
    }
}

#[test]
fn search_command_returns_search_v1_schema() {
    let (ok, stdout, _) = run(&["search", "--json", "anything"]);
    assert!(ok);
    let json = parse_json(&stdout);
    assert_eq!(json["schema_version"], "lazyadmin.search.v1");
    // SearchResults shape: typed groups keyed by entity kind.
    for key in [
        "listeners",
        "processes",
        "workloads",
        "projects",
        "managers",
    ] {
        assert!(
            json.get(key).is_some(),
            "search results missing key `{key}`:\n{json}"
        );
        assert!(json[key]["hits"].is_array(), "`{key}.hits` not array");
    }
}

#[test]
fn search_command_rejects_zero_limit() {
    let (ok, _stdout, stderr) = run(&["search", "--json", "--limit", "0", "x"]);
    assert!(!ok, "limit 0 must be rejected");
    assert!(
        stderr.to_lowercase().contains("limit"),
        "expected limit error in stderr: {stderr}"
    );
}

#[test]
fn diff_against_empty_snapshot_succeeds_with_diff_schema_v1() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let empty = format!("{manifest}/../../testdata/snapshots/empty.json");
    let (ok, stdout, _) = run(&["diff", "--json", &empty, &empty]);
    assert!(ok, "diff between empty snapshots should succeed");
    let json = parse_json(&stdout);
    assert_eq!(json["schema_version"], "lazyadmin.diff.v1");
    assert!(json["listeners"]["added"].is_array());
}

#[test]
fn diff_of_empty_vs_busy_reports_added_listeners() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let empty = format!("{manifest}/../../testdata/snapshots/empty.json");
    let busy = format!("{manifest}/../../testdata/snapshots/busy.json");
    let (ok, stdout, _) = run(&["diff", "--json", &empty, &busy]);
    assert!(ok);
    let json = parse_json(&stdout);
    let added = json["listeners"]["added"]
        .as_array()
        .expect("listeners.added array");
    assert!(
        !added.is_empty(),
        "expected listeners added when diffing empty -> busy"
    );
}

#[test]
fn events_once_json_returns_array() {
    let (ok, stdout, _) = run(&["events", "--once", "--json"]);
    assert!(ok);
    let json = parse_json(&stdout);
    // --once mode emits an array of events.
    assert!(
        json.is_array() || json.is_object(),
        "events --once must be JSON"
    );
}

#[test]
fn tui_headless_json_returns_view_object() {
    let (ok, stdout, _) = run(&["tui", "--headless", "--json"]);
    assert!(ok, "tui --headless --json should succeed");
    let json = parse_json(&stdout);
    assert!(
        json.is_object(),
        "tui headless output should be a JSON object"
    );
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let (ok, _stdout, stderr) = run(&["definitely-not-a-command"]);
    assert!(!ok);
    assert!(!stderr.is_empty(), "stderr should describe the error");
}

#[test]
fn port_flag_rejects_subcommand() {
    let (ok, _stdout, stderr) = run(&["--port", "0", "export", "--json"]);
    assert!(!ok, "--port must not silently override a subcommand");
    assert!(
        stderr.contains("--port cannot be combined with a subcommand"),
        "expected --port conflict error in stderr: {stderr}"
    );
}

#[test]
fn invalid_diff_path_exits_nonzero() {
    let (ok, _stdout, stderr) = run(&["diff", "/no/such/file.a", "/no/such/file.b"]);
    assert!(!ok);
    assert!(!stderr.is_empty());
}
