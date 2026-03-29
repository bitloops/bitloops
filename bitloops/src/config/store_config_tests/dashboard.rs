use super::*;

#[test]
fn resolve_dashboard_config_reads_repo_config_via_public_helper() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "dashboard": {
                "local_dashboard": {
                    "tls": true
                }
            }
        }),
    );

    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert_eq!(
        resolve_dashboard_config().local_dashboard,
        Some(DashboardLocalDashboardConfig { tls: Some(true) })
    );
}

#[test]
fn dashboard_file_config_load_defaults_when_repo_config_missing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert_eq!(DashboardFileConfig::load(), DashboardFileConfig::default());
    assert_eq!(resolve_dashboard_config(), DashboardFileConfig::default());
}

#[test]
fn dashboard_file_config_reads_local_dashboard_flags() {
    let value = serde_json::json!({
        "dashboard": {
            "local_dashboard": {
                "tls": true
            }
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(
        cfg.local_dashboard,
        Some(DashboardLocalDashboardConfig { tls: Some(true) })
    );
}

#[test]
fn dashboard_file_config_defaults_when_dashboard_block_missing() {
    let value = serde_json::json!({
        "stores": {
            "relational": {
                "sqlite_path": "data/relational.db"
            }
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg, DashboardFileConfig::default());
}

#[test]
fn dashboard_file_config_accepts_boolean_like_strings() {
    let value = serde_json::json!({
        "dashboard": {
            "local_dashboard": {
                "tls": "yes"
            }
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(
        cfg.local_dashboard,
        Some(DashboardLocalDashboardConfig { tls: Some(true) })
    );
}

#[test]
fn dashboard_file_config_load_reads_daemon_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let config_root = temp.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root.as_str()),
        )],
    );
    write_repo_config(
        &temp.path().join("bitloops"),
        serde_json::json!({
            "dashboard": {
                "local_dashboard": {
                    "tls": true
                }
            }
        }),
    );

    let cfg = DashboardFileConfig::load();
    assert_eq!(
        cfg.local_dashboard,
        Some(DashboardLocalDashboardConfig { tls: Some(true) })
    );
}

#[test]
fn resolve_dashboard_config_reads_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "dashboard": {
                "local_dashboard": {
                    "tls": true
                }
            }
        }),
    );

    with_cwd(temp.path(), || {
        assert_eq!(
            resolve_dashboard_config().local_dashboard,
            Some(DashboardLocalDashboardConfig { tls: Some(true) })
        );
    });
}
