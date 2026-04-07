use super::*;

#[test]
fn parse_sync_source_accepts_default_and_aliases() {
    assert_eq!(
        parse_sync_source(None).expect("default source"),
        crate::daemon::SyncTaskSource::ManualCli
    );
    assert_eq!(
        parse_sync_source(Some("   ")).expect("blank source"),
        crate::daemon::SyncTaskSource::ManualCli
    );
    assert_eq!(
        parse_sync_source(Some("manual")).expect("manual alias"),
        crate::daemon::SyncTaskSource::ManualCli
    );
    assert_eq!(
        parse_sync_source(Some("manual-cli")).expect("manual-cli alias"),
        crate::daemon::SyncTaskSource::ManualCli
    );
    assert_eq!(
        parse_sync_source(Some("init")).expect("init source"),
        crate::daemon::SyncTaskSource::Init
    );
    assert_eq!(
        parse_sync_source(Some("watcher")).expect("watcher source"),
        crate::daemon::SyncTaskSource::Watcher
    );
    assert_eq!(
        parse_sync_source(Some("post-commit")).expect("post-commit source"),
        crate::daemon::SyncTaskSource::PostCommit
    );
    assert_eq!(
        parse_sync_source(Some("post_merge")).expect("post_merge source"),
        crate::daemon::SyncTaskSource::PostMerge
    );
    assert_eq!(
        parse_sync_source(Some("post_checkout")).expect("post_checkout source"),
        crate::daemon::SyncTaskSource::PostCheckout
    );
}

#[test]
fn parse_sync_source_rejects_unknown_values() {
    let err = parse_sync_source(Some("cronjob")).expect_err("unknown source should fail");
    assert!(err.contains("unsupported sync source `cronjob`"));
    assert!(err.contains("manual_cli"));
}

#[test]
fn resolve_sync_mode_input_defaults_to_auto_when_no_selector_is_set() {
    let mode = resolve_sync_mode_input(false, None, false, false, "sync").expect("default mode");
    assert_eq!(mode, crate::host::devql::SyncMode::Auto);
}

#[test]
fn resolve_sync_mode_input_rejects_conflicting_selectors() {
    let err = resolve_sync_mode_input(
        true,
        Some(vec!["src/lib.rs".to_string()]),
        false,
        false,
        "enqueueSync",
    )
    .expect_err("conflicting selectors should fail");
    assert!(
        err.message
            .contains("at most one of `full`, `paths`, `repair`, or `validate` may be specified")
    );
}

#[test]
fn to_graphql_count_clamps_large_values() {
    assert_eq!(to_graphql_count(0), 0);
    assert_eq!(to_graphql_count(42), 42);
    assert_eq!(
        to_graphql_count((i32::MAX as usize) + 10),
        i32::MAX,
        "values larger than i32::MAX should clamp"
    );
}

#[test]
fn require_non_empty_input_trims_and_rejects_blank_values() {
    let value =
        require_non_empty_input("  hello  ".to_string(), "field", "operation").expect("trim");
    assert_eq!(value, "hello");

    let err = require_non_empty_input("   ".to_string(), "field", "operation")
        .expect_err("blank input should fail");
    let message = err.message.clone();
    assert!(message.contains("field must not be empty"));
}
