use super::*;

#[test]
fn dashboard_use_bitloops_local_reads_repo_config_via_public_helper() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "dashboard": {
                "use_bitloops_local": true
            }
        }),
    );

    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert!(dashboard_use_bitloops_local());
}

#[test]
fn dashboard_file_config_load_defaults_when_repo_config_missing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    assert_eq!(DashboardFileConfig::load(), DashboardFileConfig::default());
    assert!(!dashboard_use_bitloops_local());
}

#[test]
fn dashboard_file_config_reads_use_bitloops_local_flag() {
    let value = serde_json::json!({
        "dashboard": {
            "use_bitloops_local": true
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg.use_bitloops_local, Some(true));
}

#[test]
fn dashboard_file_config_defaults_when_dashboard_block_missing() {
    let value = serde_json::json!({
        "stores": {
            "relational": {
                "provider": "sqlite"
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
            "use_bitloops_local": "yes"
        }
    });

    let cfg = DashboardFileConfig::from_json_value(&value);
    assert_eq!(cfg.use_bitloops_local, Some(true));
}

#[test]
fn dashboard_file_config_load_reads_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_repo_config(
        temp.path(),
        serde_json::json!({
            "dashboard": {
                "use_bitloops_local": true
            }
        }),
    );

    with_cwd(temp.path(), || {
        let cfg = DashboardFileConfig::load();
        assert_eq!(cfg.use_bitloops_local, Some(true));
    });
}

#[test]
fn dashboard_use_bitloops_local_reads_repo_config_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    write_envelope_config(
        temp.path(),
        serde_json::json!({
            "dashboard": {
                "use_bitloops_local": true
            }
        }),
    );

    with_cwd(temp.path(), || {
        assert!(dashboard_use_bitloops_local());
    });
}
