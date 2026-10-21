use super::*;
use serde_json::json;

#[test]
fn configuration_errors_do_not_echo_untrusted_wrapper_fields() {
    let path =
        std::env::temp_dir().join(format!("mihomo-doctor-invalid-{}.yaml", std::process::id()));
    std::fs::write(
        &path,
        "kind: mihomo-tui/v1\nbackend: fixture-secret\nprofile: {}\n",
    )
    .unwrap();
    let error = read_configuration(&path).unwrap_err();
    std::fs::remove_file(path).unwrap();
    assert!(!error.contains("fixture-secret"));
}

#[test]
fn explicit_controllers_do_not_inherit_local_configuration_secrets() {
    let path = std::env::temp_dir().join(format!(
        "mihomo-doctor-credentials-{}.yaml",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "external-controller: 127.0.0.1:9090\nsecret: saved-secret\n",
    )
    .unwrap();
    let client = client_for(&path, Some("http://127.0.0.1:9093".into()), None).unwrap();
    assert_eq!(client.redact("saved-secret"), "saved-secret");
    let local = client_for(&path, None, None).unwrap();
    assert!(!local.redact("saved-secret").contains("saved-secret"));
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn diagnostics_refuse_non_regular_config_sources() {
    let path = std::env::temp_dir().join(format!("mihomo-doctor-link-{}", std::process::id()));
    std::os::unix::fs::symlink("/dev/null", &path).unwrap();
    let result = read_configuration(&path);
    std::fs::remove_file(path).unwrap();
    assert!(result.unwrap_err().contains("普通文件"));
}

#[test]
fn external_controller_does_not_inspect_local_services_or_create_configuration() {
    let path = std::env::temp_dir().join("mihomo-doctor-external-missing/config.yaml");
    let report = collect(&Options {
        path: path.clone(),
        client: None,
        external: true,
    });
    assert_eq!(report.scope, "external_controller");
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.status == Status::NotApplicable)
    );
    assert!(!report.checks.iter().any(|check| check.id == "service"));
    assert!(!path.exists());
    assert!(!report.has_failures());
}

#[test]
fn a_route_without_a_policy_entry_is_not_a_working_tun() {
    let checks = evaluate_routes(
        "Meta",
        2022,
        &json!([]),
        &json!([
            {"dst":"default", "gateway":"198.18.0.2", "dev":"Meta"}
        ]),
    );
    assert_eq!(checks[0].status, Status::Fail);
    assert_eq!(checks[1].status, Status::Pass);
}

#[test]
fn unresolved_goto_is_reported_even_when_a_lookup_rule_exists() {
    let checks = evaluate_routes(
        "Meta",
        2022,
        &json!([
            {"priority":9001, "iif":"Meta", "goto":9010},
            {"priority":9002, "table":"2022"}
        ]),
        &json!([]),
    );
    assert_eq!(checks[0].status, Status::Fail);
    assert!(checks[0].detail.contains("9010"));
}

#[test]
fn custom_tables_and_a_resolved_goto_pass_only_structural_checks() {
    let checks = evaluate_routes(
        "custom-tun",
        3022,
        &json!([
            {"priority":8001, "goto":8010},
            {"priority":8002, "table":3022},
            {"priority":8010, "nop":null}
        ]),
        &json!([{"dst":"default", "dev":"custom-tun"}]),
    );
    assert!(checks.iter().all(|check| check.status == Status::Pass));
    assert!(!checks.iter().any(|check| check.detail.contains("接管成功")));
}

#[test]
fn wrong_interface_routes_are_not_accepted() {
    let checks = evaluate_routes(
        "Meta",
        2022,
        &json!([{"priority":9002,"table":2022}]),
        &json!([{"dst":"default","dev":"ens34"}]),
    );
    assert_eq!(checks[1].status, Status::Fail);
}

#[test]
fn missing_config_is_never_created_by_diagnostics() {
    let path = std::env::temp_dir()
        .join(format!("mihomo-doctor-missing-{}", std::process::id()))
        .join("config.yaml");
    assert!(read_configuration(&path).is_err());
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn malformed_route_evidence_is_unknown_not_healthy_or_broken() {
    let checks = evaluate_routes("Meta", 2022, &json!({}), &json!({}));
    assert!(checks.iter().all(|check| check.status == Status::Unknown));
}
