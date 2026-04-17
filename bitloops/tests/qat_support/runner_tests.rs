use super::*;
use cucumber::event::{ScenarioFinished, StepError};
use cucumber::gherkin::{Feature, LineCol, Rule, Scenario, Span};

#[test]
fn prepare_suite_binary_copies_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let binary_path = temp.path().join("bitloops");
    fs::write(&binary_path, b"qat-test-binary").expect("write source binary");
    let suite_root = temp.path().join("suite");
    fs::create_dir_all(&suite_root).expect("create suite root");

    let snapshot_binary =
        prepare_suite_binary(&binary_path, &suite_root).expect("prepare suite binary");
    assert_eq!(snapshot_binary, suite_root.join("bitloops"));
    assert!(snapshot_binary.exists());
    assert_eq!(
        fs::read(&snapshot_binary).expect("read snapshot"),
        b"qat-test-binary"
    );
}

#[test]
fn resolve_execution_binary_uses_snapshot_for_onboarding() {
    let original = PathBuf::from("/tmp/original-bitloops");
    let snapshot = PathBuf::from("/tmp/suite/bitloops");
    assert_eq!(
        resolve_execution_binary(&Suite::Onboarding, &original, &snapshot),
        snapshot
    );
}

#[test]
fn resolve_execution_binary_uses_snapshot_for_devql_sync() {
    let original = PathBuf::from("/tmp/original-bitloops");
    let snapshot = PathBuf::from("/tmp/suite/bitloops");
    assert_eq!(
        resolve_execution_binary(&Suite::DevqlSync, &original, &snapshot),
        snapshot
    );
}

#[test]
fn agents_checkpoints_suite_reports_expected_id_and_rerun_alias() {
    assert_eq!(Suite::AgentsCheckpoints.id(), "agents-checkpoints");
    assert_eq!(
        Suite::AgentsCheckpoints.rerun_alias(),
        "cargo qat-agents-checkpoints"
    );
}

#[test]
fn suite_feature_path_points_to_agents_checkpoints_feature() {
    let path = suite_feature_path(&Suite::AgentsCheckpoints);
    assert_eq!(
        path,
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("qat")
            .join("features")
            .join("agents-checkpoints")
    );
}

#[test]
fn suite_feature_path_points_to_dedicated_devql_ingest_feature() {
    let path = suite_feature_path(&Suite::DevqlIngest);
    assert_eq!(
        path,
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("qat")
            .join("features")
            .join("devql-ingest")
            .join("ingest_workspace.feature")
    );
}

#[test]
fn parse_cucumber_tags_filter_treats_missing_or_blank_values_as_disabled() {
    assert!(parse_cucumber_tags_filter(None).unwrap().is_none());
    assert!(parse_cucumber_tags_filter(Some("")).unwrap().is_none());
    assert!(parse_cucumber_tags_filter(Some("   ")).unwrap().is_none());
}

#[test]
fn parse_cucumber_tags_filter_accepts_valid_tag_expression() {
    let parsed = parse_cucumber_tags_filter(Some("@test_harness_sync and not @slow"))
        .expect("parse tag filter")
        .expect("tag filter should be present");

    assert!(parsed.eval(["test_harness_sync"]));
    assert!(!parsed.eval(["test_harness_sync", "slow"]));
}

#[test]
fn scenario_matches_tags_filter_merges_feature_rule_and_scenario_tags() {
    let feature = Feature {
        keyword: "Feature".to_string(),
        name: "feature".to_string(),
        description: None,
        background: None,
        scenarios: Vec::new(),
        rules: Vec::new(),
        tags: vec!["feature_tag".to_string()],
        span: Span::default(),
        position: LineCol::default(),
        path: None,
    };
    let rule = Rule {
        keyword: "Rule".to_string(),
        name: "rule".to_string(),
        description: None,
        background: None,
        scenarios: Vec::new(),
        tags: vec!["rule_tag".to_string()],
        span: Span::default(),
        position: LineCol::default(),
    };
    let scenario = Scenario {
        keyword: "Scenario".to_string(),
        name: "scenario".to_string(),
        description: None,
        steps: Vec::new(),
        examples: Vec::new(),
        tags: vec!["scenario_tag".to_string()],
        span: Span::default(),
        position: LineCol::default(),
    };
    let filter = parse_cucumber_tags_filter(Some("@feature_tag and @rule_tag and @scenario_tag"))
        .expect("parse tag filter")
        .expect("tag filter should be present");

    assert!(scenario_matches_tags_filter(
        &feature,
        Some(&rule),
        &scenario,
        &filter
    ));
}

#[test]
fn failed_scenario_from_event_captures_failed_scenario_location() {
    let feature = Feature {
        keyword: "Feature".to_string(),
        name: "feature".to_string(),
        description: None,
        background: None,
        scenarios: Vec::new(),
        rules: Vec::new(),
        tags: Vec::new(),
        span: Span::default(),
        position: LineCol::default(),
        path: Some(PathBuf::from(
            "qat/features/devql-sync/sync_workspace.feature",
        )),
    };
    let scenario = Scenario {
        keyword: "Scenario".to_string(),
        name: "Sync removes artefacts for deleted source files".to_string(),
        description: None,
        steps: Vec::new(),
        examples: Vec::new(),
        tags: Vec::new(),
        span: Span::default(),
        position: LineCol { line: 94, col: 5 },
    };

    let failed = failed_scenario_from_event(
        &feature,
        &scenario,
        &ScenarioFinished::StepFailed(None, None, StepError::NotFound),
    )
    .expect("failed scenario should be captured");

    assert_eq!(
        failed,
        FailedScenario {
            name: "Sync removes artefacts for deleted source files".to_string(),
            location: Some("qat/features/devql-sync/sync_workspace.feature:94".to_string()),
        }
    );
}

#[test]
fn failed_scenario_from_event_ignores_passing_scenarios() {
    let feature = Feature {
        keyword: "Feature".to_string(),
        name: "feature".to_string(),
        description: None,
        background: None,
        scenarios: Vec::new(),
        rules: Vec::new(),
        tags: Vec::new(),
        span: Span::default(),
        position: LineCol::default(),
        path: Some(PathBuf::from(
            "qat/features/devql-sync/sync_workspace.feature",
        )),
    };
    let scenario = Scenario {
        keyword: "Scenario".to_string(),
        name: "Sync removes artefacts for deleted source files".to_string(),
        description: None,
        steps: Vec::new(),
        examples: Vec::new(),
        tags: Vec::new(),
        span: Span::default(),
        position: LineCol { line: 94, col: 5 },
    };

    assert!(
        failed_scenario_from_event(&feature, &scenario, &ScenarioFinished::StepPassed).is_none(),
        "passing scenarios should not appear in the failure summary"
    );
}

#[test]
fn build_suite_failure_message_lists_failed_scenarios() {
    let message = build_suite_failure_message(
        &Suite::DevqlSync,
        Path::new("qat/features/devql-sync/sync_workspace.feature"),
        0,
        0,
        Path::new("/tmp/qat-run"),
        &[
            FailedScenario {
                name: "Sync removes artefacts for deleted source files".to_string(),
                location: Some("qat/features/devql-sync/sync_workspace.feature:94".to_string()),
            },
            FailedScenario {
                name: "Sync catches up after daemon downtime with accumulated changes".to_string(),
                location: Some("qat/features/devql-sync/sync_workspace.feature:145".to_string()),
            },
        ],
    );

    assert!(
        message.contains("failed_scenarios:"),
        "suite failure message should include a failed_scenarios section: {message}"
    );
    assert!(
        message.contains(
            "- Sync removes artefacts for deleted source files (qat/features/devql-sync/sync_workspace.feature:94)"
        ),
        "suite failure message should list the first failed scenario: {message}"
    );
    assert!(
        message.contains(
            "- Sync catches up after daemon downtime with accumulated changes (qat/features/devql-sync/sync_workspace.feature:145)"
        ),
        "suite failure message should list the second failed scenario: {message}"
    );
}
