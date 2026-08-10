use crate::{
    core::{CoreRelease, CoreVersion},
    core_manager::{
        CorePaths, binary_version, install_version, managed_active_version, switch_current,
    },
    core_package,
    mihomo::MihomoClient,
    runtime,
    system::{
        acquire_runtime_lock, checked_output, clean_command, effective_root,
        trusted_root_directory, trusted_root_file,
    },
    workspace,
};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const SYSTEM_BINARIES: [&str; 2] = ["/usr/bin/mihomo", "/usr/local/bin/mihomo"];
const HEALTH_ATTEMPTS: usize = 6;
const HEALTH_RETRY_DELAY: Duration = Duration::from_millis(250);

trait ActivationOperations {
    fn switch(&mut self, version: CoreVersion) -> Result<(), String>;
    fn configure_service(&mut self) -> Result<(), String>;
    fn restart_service(&mut self) -> Result<(), String>;
    fn health(&mut self, expected: CoreVersion) -> Result<(), String>;
}

fn activate_with_rollback(
    operations: &mut impl ActivationOperations,
    previous: CoreVersion,
    candidate: CoreVersion,
) -> Result<(), String> {
    let activate = activate_version(operations, candidate);
    let Err(activation_error) = activate else {
        return Ok(());
    };

    match activate_version(operations, previous) {
        Ok(()) => Err(format!(
            "managed core activation failed and was rolled back to {previous}: {activation_error}"
        )),
        Err(rollback_error) => Err(format!(
            "managed core activation failed: {activation_error}; rollback to {previous} also failed: {rollback_error}"
        )),
    }
}

fn activate_version(
    operations: &mut impl ActivationOperations,
    version: CoreVersion,
) -> Result<(), String> {
    operations.switch(version)?;
    operations.configure_service()?;
    operations.restart_service()?;
    operations.health(version)
}

struct HostActivation {
    paths: CorePaths,
    client: MihomoClient,
}

impl ActivationOperations for HostActivation {
    fn switch(&mut self, version: CoreVersion) -> Result<(), String> {
        switch_current(&self.paths, version)
    }

    fn configure_service(&mut self) -> Result<(), String> {
        runtime::configure_managed_core_service()
    }

    fn restart_service(&mut self) -> Result<(), String> {
        runtime::restart_service()
    }

    fn health(&mut self, expected: CoreVersion) -> Result<(), String> {
        let mut last_error = "health check was not attempted".to_string();
        for attempt in 0..HEALTH_ATTEMPTS {
            match self.client.version() {
                Ok(actual) if actual == expected => match self.client.proxies() {
                    Ok(_) => return Ok(()),
                    Err(error) => last_error = format!("/proxies failed: {error}"),
                },
                Ok(actual) => {
                    last_error = format!("/version expected {expected}, received {actual}")
                }
                Err(error) => last_error = format!("/version failed: {error}"),
            }
            if attempt + 1 < HEALTH_ATTEMPTS {
                thread::sleep(HEALTH_RETRY_DELAY);
            }
        }
        Err(format!(
            "Mihomo controller did not become healthy after {HEALTH_ATTEMPTS} attempts: {last_error}"
        ))
    }
}

pub fn upgrade() -> Result<String, String> {
    if !effective_root() {
        return Err(runtime::root_required_message().into());
    }
    let _lock = acquire_runtime_lock()?;
    let release = CoreRelease::embedded()?;
    let candidate_version = release.recommended();
    let paths = CorePaths::system();
    let active = managed_active_version(&paths)?;
    if active == Some(candidate_version) {
        return Ok(format!(
            "managed Mihomo core {candidate_version} is already up to date"
        ));
    }

    core_package::ensure_debian_host()?;
    runtime::validate_upgrade_service()?;
    validate_owned_config()?;
    let connection = crate::discovery::discover(Path::new(workspace::DEFAULT_SOURCE_PATH))
        .ok_or_else(|| {
            format!(
                "{} must define external-controller before a core upgrade",
                workspace::DEFAULT_SOURCE_PATH
            )
        })?;
    let Some(previous) = prepare_previous_core(&paths, &release, active)? else {
        return Ok(format!(
            "current Mihomo core {candidate_version} already matches the recommended version; no managed activation was needed"
        ));
    };

    if let Some(candidate) = installed_candidate(&paths, candidate_version)? {
        eprintln!("validating bundled Mihomo {candidate_version}...");
        validate_candidate(&candidate, candidate_version)?;
    } else {
        let package = release.package_for(std::env::consts::OS, std::env::consts::ARCH)?;
        eprintln!("downloading and verifying official Mihomo {candidate_version}...");
        let downloaded = core_package::download_package(&release, package)?;
        let extracted = core_package::extract_core(&downloaded)?;
        validate_candidate(extracted.candidate(), candidate_version)?;
        install_version(&paths, extracted.candidate(), candidate_version)?;
    }

    let client = MihomoClient::new(connection.controller, connection.secret)
        .map_err(|error| format!("cannot create Mihomo health client: {error}"))?;
    let mut activation = HostActivation { paths, client };
    activate_with_rollback(&mut activation, previous, candidate_version)?;
    Ok(format!(
        "managed Mihomo core upgraded from {previous} to {candidate_version}"
    ))
}

fn installed_candidate(
    paths: &CorePaths,
    expected: CoreVersion,
) -> Result<Option<PathBuf>, String> {
    managed_active_version(paths)?;
    let binary = paths.binary(expected);
    match fs::symlink_metadata(&binary) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect installed managed core {}: {error}",
            binary.display()
        )),
        Ok(_) if !trusted_root_file(&binary) => Err(format!(
            "installed managed core {} is not a trusted root file",
            binary.display()
        )),
        Ok(_) if binary_version(&binary)? != expected => Err(format!(
            "installed managed core {} does not contain {expected}",
            binary.display()
        )),
        Ok(_) => Ok(Some(binary)),
    }
}

fn prepare_previous_core(
    paths: &CorePaths,
    release: &CoreRelease,
    active: Option<CoreVersion>,
) -> Result<Option<CoreVersion>, String> {
    if let Some(version) = active {
        return rollback_version(release, version);
    }

    let (binary, version) = find_system_core()?;
    let rollback = rollback_version(release, version)?;
    if rollback.is_none() {
        return Ok(None);
    }
    install_version(paths, &binary, version)?;
    Ok(rollback)
}

fn rollback_version(
    release: &CoreRelease,
    version: CoreVersion,
) -> Result<Option<CoreVersion>, String> {
    release.require_supported(version)?;
    Ok((version != release.recommended()).then_some(version))
}

fn find_system_core() -> Result<(PathBuf, CoreVersion), String> {
    for candidate in SYSTEM_BINARIES.map(PathBuf::from) {
        match fs::symlink_metadata(&candidate) {
            Ok(_) if !trusted_root_file(&candidate) => {
                return Err(format!(
                    "system Mihomo binary {} is not a trusted root file",
                    candidate.display()
                ));
            }
            Ok(_) => return Ok((candidate.clone(), binary_version(&candidate)?)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect system Mihomo binary {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Err("no trusted current Mihomo core is available for rollback".into())
}

fn validate_owned_config() -> Result<(), String> {
    let config = Path::new(workspace::DEFAULT_SOURCE_PATH);
    let data_dir = config
        .parent()
        .ok_or_else(|| "owned Mihomo config has no data directory".to_string())?;
    if !trusted_root_directory(data_dir) {
        return Err(format!(
            "owned Mihomo data directory {} is not a trusted root directory",
            data_dir.display()
        ));
    }
    if !trusted_root_file(config) {
        return Err(format!(
            "owned Mihomo config {} is not a trusted root file",
            config.display()
        ));
    }
    Ok(())
}

fn validate_candidate(candidate: &Path, expected: CoreVersion) -> Result<(), String> {
    let actual = binary_version(candidate)?;
    if actual != expected {
        return Err(format!(
            "extracted Mihomo version mismatch: expected {expected}, received {actual}"
        ));
    }
    let data_dir = Path::new(workspace::DEFAULT_SOURCE_PATH)
        .parent()
        .ok_or_else(|| "owned Mihomo config has no data directory".to_string())?;
    let output = clean_command(candidate)
        .args(["-t", "-d"])
        .arg(data_dir)
        .output()
        .map_err(|error| format!("cannot validate config with candidate Mihomo: {error}"))?;
    checked_output("candidate Mihomo config validation", output).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
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

        let error =
            activate_with_rollback(&mut operations, version("v1.19.28"), version("v1.19.29"))
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

        let error =
            activate_with_rollback(&mut operations, version("v1.19.28"), version("v1.19.29"))
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
}
