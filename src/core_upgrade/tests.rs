use super::{
    activation::{ActivationOperations, HostActivation, activate_with_rollback},
    upgrade::{installed_candidate, rollback_version},
};
use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::{CorePaths, install_version},
    mihomo::MihomoClient,
};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    thread,
};

#[derive(Default)]
struct FakeActivation {
    events: Vec<String>,
    failures: Vec<String>,
}

impl FakeActivation {
    fn fail(mut self, event: &str) -> Self {
        self.failures.push(event.into());
        self
    }

    fn record(&mut self, event: String) -> Result<(), String> {
        self.events.push(event.clone());
        if self.failures.contains(&event) {
            Err(format!("{event} failed"))
        } else {
            Ok(())
        }
    }
}

impl ActivationOperations for FakeActivation {
    fn switch(&mut self, version: CoreVersion) -> Result<(), String> {
        self.record(format!("switch {version}"))
    }

    fn configure_service(&mut self) -> Result<(), String> {
        self.record("configure".into())
    }

    fn restart_service(&mut self) -> Result<(), String> {
        self.record("restart".into())
    }

    fn health(&mut self, expected: CoreVersion) -> Result<(), String> {
        self.record(format!("health {expected}"))
    }
}

fn version(value: &str) -> CoreVersion {
    CoreVersion::parse(value).unwrap()
}

#[test]
fn recommended_core_is_not_used_as_its_own_rollback() {
    let release = CoreRelease::embedded().unwrap();

    assert_eq!(
        rollback_version(&release, version("v1.19.28")).unwrap(),
        Some(version("v1.19.28"))
    );
    assert_eq!(
        rollback_version(&release, release.recommended()).unwrap(),
        None
    );
}

#[cfg(unix)]
#[test]
fn a_bundled_candidate_is_detected_for_reuse() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Path::new("/tmp").join(format!(
        "mihomo-tui-installed-candidate-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let source = root.join("mihomo");
    fs::write(
        &source,
        "#!/bin/sh\nprintf 'Mihomo Meta v1.19.29 linux amd64\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let paths = CorePaths::under(&root.join("managed"));
    let candidate = version("v1.19.29");
    install_version(&paths, &source, candidate).unwrap();

    assert_eq!(
        installed_candidate(&paths, candidate).unwrap(),
        Some(paths.binary(candidate))
    );
    assert_eq!(
        installed_candidate(&paths, version("v1.19.28")).unwrap(),
        None
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn successful_activation_checks_version_and_proxy_health() {
    let mut operations = FakeActivation::default();

    activate_with_rollback(&mut operations, version("v1.19.28"), version("v1.19.29")).unwrap();

    assert_eq!(
        operations.events,
        ["switch v1.19.29", "configure", "restart", "health v1.19.29"]
    );
}

#[test]
fn failed_candidate_health_restores_and_checks_the_previous_core() {
    let mut operations = FakeActivation::default().fail("health v1.19.29");

    let error = activate_with_rollback(&mut operations, version("v1.19.28"), version("v1.19.29"))
        .unwrap_err();

    assert!(error.contains("health v1.19.29 failed"));
    assert_eq!(
        operations.events,
        [
            "switch v1.19.29",
            "configure",
            "restart",
            "health v1.19.29",
            "switch v1.19.28",
            "configure",
            "restart",
            "health v1.19.28"
        ]
    );
}

#[test]
fn rollback_failure_is_reported_with_the_activation_failure() {
    let mut operations = FakeActivation::default()
        .fail("restart")
        .fail("switch v1.19.28");

    let error = activate_with_rollback(&mut operations, version("v1.19.28"), version("v1.19.29"))
        .unwrap_err();

    assert!(error.contains("restart failed"));
    assert!(error.contains("switch v1.19.28 failed"));
    assert!(error.contains("rollback"));
}

#[test]
fn host_health_requires_both_version_and_proxies_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut targets = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            let target = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let body = if target == "/version" {
                r#"{"meta":true,"version":"v1.19.29"}"#
            } else {
                r#"{"proxies":{"GLOBAL":{"type":"Selector","all":["DIRECT"],"now":"DIRECT"},"DIRECT":{"type":"Direct"}}}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            targets.push(target);
        }
        targets
    });
    let client = MihomoClient::new(format!("http://{address}"), None).unwrap();
    let mut activation = HostActivation {
        paths: CorePaths::under(Path::new("/unused")),
        client,
    };

    activation.health(version("v1.19.29")).unwrap();

    assert_eq!(server.join().unwrap(), ["/version", "/proxies"]);
}
