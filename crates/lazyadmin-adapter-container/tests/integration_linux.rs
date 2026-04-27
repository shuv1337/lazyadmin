#[test]
#[ignore = "requires a live Docker/Podman daemon and is optional for developer machines"]
fn docker_or_podman_available_for_manual_smoke() {
    let docker = std::path::Path::new("/var/run/docker.sock").exists();
    let podman = std::path::Path::new("/run/podman/podman.sock").exists()
        || std::env::var_os("XDG_RUNTIME_DIR")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("podman/podman.sock")
                    .exists()
            })
            .unwrap_or(false);
    if !(docker || podman) {
        eprintln!("skipping: no Docker or Podman socket available");
    }
}
