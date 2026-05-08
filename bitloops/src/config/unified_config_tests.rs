use serde_json::json;
use std::fs;

use super::unified_config::{
    ConfigScope, UnifiedSettings, load_effective_config, merge_json_layers, merge_layers,
    parse_config_envelope,
};

// ---------------------------------------------------------------------------
// A. Envelope parsing & validation
// ---------------------------------------------------------------------------

#[test]
fn parse_envelope_valid_global() {
    let data = json!({
        "version": "1.0",
        "scope": "global",
        "settings": {
            "strategy": "manual-commit"
        }
    });
    let envelope = parse_config_envelope(data.to_string().as_bytes(), ConfigScope::Global).unwrap();
    assert_eq!(envelope.version, "1.0");
    assert_eq!(envelope.scope, ConfigScope::Global);
    assert_eq!(envelope.settings.strategy.as_deref(), Some("manual-commit"));
}

#[test]
fn parse_envelope_valid_project() {
    let data = json!({
        "version": "1.0",
        "scope": "project",
        "settings": {
            "enabled": true,
            "stores": { "relational": { "sqlite_path": "data/relational.db" } }
        }
    });
    let envelope =
        parse_config_envelope(data.to_string().as_bytes(), ConfigScope::Project).unwrap();
    assert_eq!(envelope.scope, ConfigScope::Project);
    assert_eq!(envelope.settings.enabled, Some(true));
    assert!(envelope.settings.stores.is_some());
}

#[test]
fn parse_envelope_valid_project_local() {
    let data = json!({
        "version": "1.0",
        "scope": "project_local",
        "settings": {
            "enabled": false
        }
    });
    let envelope =
        parse_config_envelope(data.to_string().as_bytes(), ConfigScope::ProjectLocal).unwrap();
    assert_eq!(envelope.scope, ConfigScope::ProjectLocal);
    assert_eq!(envelope.settings.enabled, Some(false));
}

#[test]
fn parse_envelope_rejects_unknown_top_level_key() {
    let data = json!({
        "version": "1.0",
        "scope": "project",
        "settings": {},
        "bogus_key": true
    });
    let err = parse_config_envelope(data.to_string().as_bytes(), ConfigScope::Project)
        .expect_err("should reject unknown top-level key");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unknown field"),
        "error should mention unknown field, got: {msg}"
    );
}

#[test]
fn parse_envelope_rejects_unknown_settings_key() {
    let data = json!({
        "version": "1.0",
        "scope": "project",
        "settings": {
            "not_a_real_key": 42
        }
    });
    let err = parse_config_envelope(data.to_string().as_bytes(), ConfigScope::Project)
        .expect_err("should reject unknown settings key");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unknown field"),
        "error should mention unknown field, got: {msg}"
    );
}

#[test]
fn parse_envelope_rejects_scope_mismatch() {
    let data = json!({
        "version": "1.0",
        "scope": "global",
        "settings": {}
    });
    let err = parse_config_envelope(data.to_string().as_bytes(), ConfigScope::Project)
        .expect_err("should reject scope mismatch");
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("scope"),
        "error should mention scope mismatch, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// B. Layer merge order
// ---------------------------------------------------------------------------

#[test]
fn merge_project_overrides_global_strategy() {
    let global = UnifiedSettings {
        strategy: Some("manual-commit".into()),
        ..Default::default()
    };
    let project = UnifiedSettings {
        strategy: Some("auto-commit".into()),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    assert_eq!(merged.strategy.as_deref(), Some("auto-commit"));
}

#[test]
fn merge_project_local_overrides_project() {
    let project = UnifiedSettings {
        strategy: Some("auto-commit".into()),
        ..Default::default()
    };
    let local = UnifiedSettings {
        strategy: Some("manual-commit".into()),
        ..Default::default()
    };
    let merged = merge_layers(&[project, local]);
    assert_eq!(merged.strategy.as_deref(), Some("manual-commit"));
}

#[test]
fn merge_three_layers_highest_wins() {
    let global = UnifiedSettings {
        strategy: Some("global-strategy".into()),
        ..Default::default()
    };
    let project = UnifiedSettings {
        strategy: Some("project-strategy".into()),
        ..Default::default()
    };
    let local = UnifiedSettings {
        strategy: Some("local-strategy".into()),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project, local]);
    assert_eq!(merged.strategy.as_deref(), Some("local-strategy"));
}

#[test]
fn merge_absent_key_inherits_from_lower_layer() {
    let global = UnifiedSettings {
        strategy: Some("from-global".into()),
        enabled: Some(true),
        ..Default::default()
    };
    let project = UnifiedSettings {
        enabled: Some(false),
        // strategy absent — should inherit from global
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    assert_eq!(
        merged.strategy.as_deref(),
        Some("from-global"),
        "absent key should fall through to lower layer"
    );
    assert_eq!(merged.enabled, Some(false));
}

#[test]
fn merge_code_defaults_used_when_all_layers_absent() {
    let global = UnifiedSettings::default();
    let project = UnifiedSettings::default();
    let merged = merge_layers(&[global, project]);
    assert_eq!(
        merged.strategy, None,
        "when no layer provides a key it should remain None"
    );
}

// ---------------------------------------------------------------------------
// C. Deep object merge
// ---------------------------------------------------------------------------

#[test]
fn merge_deep_objects_combine_keys() {
    let global = UnifiedSettings {
        stores: Some(json!({
            "relational": { "sqlite_path": "data/relational.db" }
        })),
        ..Default::default()
    };
    let project = UnifiedSettings {
        stores: Some(json!({
            "events": { "duckdb_path": "data/events.duckdb" }
        })),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let stores = merged.stores.expect("stores should be present");
    assert!(
        stores.get("relational").is_some(),
        "relational from global should survive deep merge"
    );
    assert!(
        stores.get("events").is_some(),
        "events from project should be added"
    );
}

#[test]
fn merge_deep_objects_nested_override() {
    let global = UnifiedSettings {
        stores: Some(json!({
            "relational": { "sqlite_path": "/tmp/a.db", "postgres_dsn": "postgres://old/db" }
        })),
        ..Default::default()
    };
    let project = UnifiedSettings {
        stores: Some(json!({
            "relational": { "postgres_dsn": "postgres://new/db" }
        })),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let stores = merged.stores.expect("stores should be present");
    let rel = stores.get("relational").expect("relational should exist");
    assert_eq!(
        rel.get("postgres_dsn").and_then(|v| v.as_str()),
        Some("postgres://new/db"),
        "project should override nested postgres_dsn"
    );
    assert_eq!(
        rel.get("sqlite_path").and_then(|v| v.as_str()),
        Some("/tmp/a.db"),
        "unmodified sibling key should survive deep merge"
    );
}

#[test]
fn merge_strategy_options_deep_merge() {
    let global = UnifiedSettings {
        strategy_options: Some([("key1".into(), json!("val1"))].into_iter().collect()),
        ..Default::default()
    };
    let project = UnifiedSettings {
        strategy_options: Some([("key2".into(), json!("val2"))].into_iter().collect()),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let opts = merged
        .strategy_options
        .expect("strategy_options should be present");
    assert!(opts.contains_key("key1"), "key1 from global should survive");
    assert!(
        opts.contains_key("key2"),
        "key2 from project should be added"
    );
}

// ---------------------------------------------------------------------------
// D. Array replacement
// ---------------------------------------------------------------------------

#[test]
fn merge_array_replaces_entirely() {
    let global = UnifiedSettings {
        stores: Some(json!({
            "tags": ["alpha", "beta"]
        })),
        ..Default::default()
    };
    let project = UnifiedSettings {
        stores: Some(json!({
            "tags": ["gamma"]
        })),
        ..Default::default()
    };
    let merged = merge_layers(&[global, project]);
    let stores = merged.stores.expect("stores should be present");
    let tags = stores.get("tags").expect("tags should exist");
    assert_eq!(
        tags,
        &json!(["gamma"]),
        "array should be replaced entirely, not appended"
    );
}

// ---------------------------------------------------------------------------
// E. Null behavior
// ---------------------------------------------------------------------------

#[test]
fn merge_null_clears_lower_layer_value() {
    // Null semantics require raw JSON layers (typed Option::None is indistinguishable
    // from absent, so merge_json_layers is the correct API for null handling).
    let global = json!({ "strategy": "auto-commit" });
    let project = json!({ "strategy": null });
    let merged = merge_json_layers(&[global, project]).unwrap();
    assert_eq!(
        merged.strategy, None,
        "null in higher layer should clear the key"
    );
}

#[test]
fn merge_null_clears_nested_object() {
    let global = json!({ "stores": { "relational": { "sqlite_path": "data/relational.db" } } });
    let local = json!({ "stores": null });
    let merged = merge_json_layers(&[global, local]).unwrap();
    assert_eq!(
        merged.stores, None,
        "null in higher layer should clear the entire stores block"
    );
}

// ---------------------------------------------------------------------------
// F. Enabled in same pipeline
// ---------------------------------------------------------------------------

#[test]
fn merge_enabled_overrides_from_project_local() {
    let global = UnifiedSettings {
        enabled: Some(true),
        ..Default::default()
    };
    let local = UnifiedSettings {
        enabled: Some(false),
        ..Default::default()
    };
    let merged = merge_layers(&[global, local]);
    assert_eq!(
        merged.enabled,
        Some(false),
        "project_local enabled=false should override global enabled=true"
    );
}

#[test]
fn merge_enabled_coexists_with_stores() {
    let global = UnifiedSettings {
        enabled: Some(true),
        stores: Some(json!({ "relational": { "sqlite_path": "data/relational.db" } })),
        ..Default::default()
    };
    let local = UnifiedSettings {
        enabled: Some(false),
        ..Default::default()
    };
    let merged = merge_layers(&[global, local]);
    assert_eq!(merged.enabled, Some(false));
    assert!(
        merged.stores.is_some(),
        "stores from global should survive when local only overrides enabled"
    );
}

// ---------------------------------------------------------------------------
// G. File-based loading (integration)
// ---------------------------------------------------------------------------

fn write_config_file(dir: &std::path::Path, filename: &str, value: serde_json::Value) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(filename),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}

#[test]
fn load_effective_merges_global_and_project() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    write_config_file(
        home.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "global",
            "settings": {
                "strategy": "manual-commit",
                "stores": { "relational": { "sqlite_path": "data/relational.db" } }
            }
        }),
    );
    write_config_file(
        project.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "project",
            "settings": {
                "enabled": true
            }
        }),
    );

    let effective = load_effective_config(Some(home.path()), project.path()).unwrap();
    assert_eq!(effective.enabled, Some(true));
    assert_eq!(effective.strategy.as_deref(), Some("manual-commit"));
    assert!(effective.stores.is_some());
}

#[test]
fn load_effective_merges_all_three_scopes() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    write_config_file(
        home.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "global",
            "settings": {
                "strategy": "global-strategy",
                "enabled": true
            }
        }),
    );
    write_config_file(
        project.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "project",
            "settings": {
                "strategy": "project-strategy"
            }
        }),
    );
    write_config_file(
        project.path(),
        "config.local.json",
        json!({
            "version": "1.0",
            "scope": "project_local",
            "settings": {
                "strategy": "local-strategy"
            }
        }),
    );

    let effective = load_effective_config(Some(home.path()), project.path()).unwrap();
    assert_eq!(
        effective.strategy.as_deref(),
        Some("local-strategy"),
        "project_local should win"
    );
    assert_eq!(
        effective.enabled,
        Some(true),
        "enabled from global should propagate"
    );
}

#[test]
fn load_effective_missing_global_still_works() {
    let home = tempfile::tempdir().unwrap(); // no config written
    let project = tempfile::tempdir().unwrap();

    write_config_file(
        project.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "project",
            "settings": {
                "enabled": true,
                "strategy": "auto-commit"
            }
        }),
    );

    let effective = load_effective_config(Some(home.path()), project.path()).unwrap();
    assert_eq!(effective.enabled, Some(true));
    assert_eq!(effective.strategy.as_deref(), Some("auto-commit"));
}

#[test]
fn load_effective_missing_all_files_returns_defaults() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    let effective = load_effective_config(Some(home.path()), project.path()).unwrap();
    assert_eq!(effective, UnifiedSettings::default());
}

#[test]
fn load_effective_rejects_scope_mismatch_in_project_file() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    write_config_file(
        project.path(),
        "config.json",
        json!({
            "version": "1.0",
            "scope": "global",
            "settings": { "enabled": true }
        }),
    );

    let err = load_effective_config(Some(home.path()), project.path())
        .expect_err("should reject scope mismatch");
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("scope"),
        "error should mention scope mismatch, got: {msg}"
    );
}
