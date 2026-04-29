#![cfg(feature = "integration-portless")]

use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn free_portless_app() {
    let temp = tempfile::tempdir().unwrap();
    let port = free_port();
    let script = temp.path().join("fake-portless.sh");
    write_fake_portless(&script, port);

    let mut fake = Command::new(&script)
        .env("PORTLESS_STATE_DIR", temp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    wait_for(|| temp.path().join("routes.json").exists());
    wait_for(|| TcpListener::bind(("127.0.0.1", port)).is_err());

    let output = Command::new(env!("CARGO_BIN_EXE_lazyadmin"))
        .arg("--json")
        .arg("free")
        .arg(port.to_string())
        .arg("--yes-for-test-only")
        .env("PORTLESS_STATE_DIR", temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "lazyadmin free failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    wait_for(|| TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(
        !temp.path().join("routes.json").exists(),
        "fake portless cleanup did not remove route"
    );
    let _ = fake.wait();
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_fake_portless(path: &Path, port: u16) {
    fs::write(
        path,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
state="${{PORTLESS_STATE_DIR:?}}"
mkdir -p "$state"
python3 -m http.server {port} --bind 127.0.0.1 >/dev/null 2>&1 &
child=$!
printf '[{{"hostname":"fake","port":{port},"pid":%s}}]\n' "$$" > "$state/routes.json"
cleanup() {{
  kill "$child" >/dev/null 2>&1 || true
  rm -f "$state/routes.json"
  exit 0
}}
trap cleanup TERM INT
wait "$child"
"#
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for condition");
}
