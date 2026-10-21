pub const ROOT: &str = "/var/lib/mihomo-tui/network-guard";
pub const WORKER: &str = "/var/lib/mihomo-tui/network-guard/worker";
pub const SERVICE: &str = "mihomo-tui-network-recovery.service";
pub const TIMER: &str = "mihomo-tui-network-recovery.timer";
pub const SERVICE_PATH: &str = "/etc/systemd/system/mihomo-tui-network-recovery.service";
pub const TIMER_PATH: &str = "/etc/systemd/system/mihomo-tui-network-recovery.timer";
pub const HOOK_PATH: &str = "/etc/systemd/system/mihomo.service.d/20-mihomo-tui-network-guard.conf";

pub fn service_content(worker: &str) -> String {
    format!(
        "# Managed by mihomo-tui network-guard protocol 1\n[Unit]\nDescription=Recover unconfirmed Mihomo configuration\nAfter=local-fs.target\n\n[Service]\nType=oneshot\nExecStart={worker} network _recover\nUser=root\nGroup=root\nUMask=0077\nNoNewPrivileges=yes\nTimeoutStartSec=10s\nTimeoutStopSec=3s\nKillMode=control-group\nRemainAfterExit=no\n"
    )
}

pub fn timer_content() -> String {
    format!(
        "# Managed by mihomo-tui network-guard protocol 1\n[Unit]\nDescription=Check unconfirmed Mihomo configuration\n\n[Timer]\nOnActiveSec=5s\nOnBootSec=5s\nOnUnitInactiveSec=5s\nAccuracySec=1s\nUnit={SERVICE}\n\n[Install]\nWantedBy=timers.target\n"
    )
}

pub fn hook_content() -> String {
    hook_content_for(WORKER)
}

fn hook_content_for(worker: &str) -> String {
    format!(
        "# Managed by mihomo-tui network-guard protocol 1\n[Service]\nExecStartPre=+{worker} network _recover --startup\n"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_units_pass_static_verification_without_registration_or_starting_services() {
        let root = super::super::store::tests::directory();
        let service = root.join(super::SERVICE);
        let timer = root.join(super::TIMER);
        let dummy = root.join("mihomo-guard-fixture.service");
        std::fs::write(&service, super::service_content("/usr/bin/true")).unwrap();
        std::fs::write(&timer, super::timer_content()).unwrap();
        std::fs::write(&dummy, format!("[Unit]\nDescription=Isolated guard syntax fixture\n[Service]\nType=simple\nExecStart=/usr/bin/true\n{}", super::hook_content_for("/usr/bin/true"))).unwrap();
        let result = std::process::Command::new("/usr/bin/systemd-analyze")
            .arg("verify")
            .args([&service, &timer, &dummy])
            .env(
                "SYSTEMD_UNIT_PATH",
                format!(
                    "{}:/usr/lib/systemd/system:/lib/systemd/system",
                    root.display()
                ),
            )
            .output()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn recovery_units_are_bounded_persistent_files_and_do_not_restart_from_the_startup_hook() {
        let service = super::service_content("/usr/bin/true");
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains("TimeoutStartSec=10s"));
        assert!(service.contains("RemainAfterExit=no"));
        let timer = super::timer_content();
        assert!(timer.contains("OnUnitInactiveSec=5s"));
        assert!(!timer.contains("Persistent=true"));
        let hook = super::hook_content();
        assert!(hook.contains("ExecStartPre=+"));
        assert!(hook.contains("--startup"));
        assert!(!hook.contains("restart"));
    }
}
