use super::*;

#[test]
fn devql_guidance_enabled_defaults_true_when_repo_policy_omits_field() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_FILE_NAME),
        r#"
[capture]
enabled = true

[agents]
supported = ["codex"]
"#,
    )
    .expect("write repo policy");

    assert!(devql_guidance_enabled(dir.path()).expect("load devql guidance flag"));
}

#[test]
fn devql_producer_settings_default_to_enabled_when_policy_omits_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_FILE_NAME),
        r#"
[capture]
enabled = true
"#,
    )
    .expect("write repo policy");

    let settings = devql_producer_settings(dir.path()).expect("load DevQL producer settings");

    assert!(settings.sync_enabled);
    assert!(settings.ingest_enabled);
}

#[test]
fn devql_producer_settings_read_local_policy_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[devql]
sync_enabled = true
ingest_enabled = false
"#,
    )
    .expect("write local repo policy");

    let settings = devql_producer_settings(dir.path()).expect("load DevQL producer settings");

    assert!(settings.sync_enabled);
    assert!(!settings.ingest_enabled);
}

#[test]
fn set_devql_producer_settings_persists_sync_and_ingest_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);

    set_devql_producer_settings(&path, true, false).expect("write DevQL producer settings");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("[devql]"));
    assert!(content.contains("sync_enabled = true"));
    assert!(content.contains("ingest_enabled = false"));
}

#[test]
fn repo_semantic_embedding_policy_reads_disabled_local_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[semantic_clones]
embedding_mode = "off"
"#,
    )
    .expect("write local repo policy");

    let policy = repo_semantic_embedding_policy(dir.path()).expect("load semantic policy");

    assert!(policy.present);
    assert_eq!(policy.embedding_mode, Some(SemanticCloneEmbeddingMode::Off));
    assert_eq!(policy.inference.code_embeddings, None);
    assert_eq!(policy.inference.summary_embeddings, None);
}

#[test]
fn repo_semantic_embedding_policy_reads_summary_mode_and_generation() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
summary_generation = "summary_local"
code_embeddings = "code_local"
"#,
    )
    .expect("write local repo policy");

    let policy = repo_semantic_embedding_policy(dir.path()).expect("load semantic policy");

    assert!(policy.present);
    assert_eq!(policy.summary_mode, Some(SemanticSummaryMode::Auto));
    assert_eq!(
        policy.embedding_mode,
        Some(SemanticCloneEmbeddingMode::SemanticAwareOnce)
    );
    assert_eq!(
        policy.inference.summary_generation.as_deref(),
        Some("summary_local")
    );
    assert_eq!(
        policy.inference.code_embeddings.as_deref(),
        Some("code_local")
    );
    assert_eq!(policy.inference.summary_embeddings, None);
}

#[test]
fn set_repo_semantic_embedding_policy_persists_profile_bindings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);

    set_repo_semantic_embedding_policy(
        &path,
        &RepoSemanticEmbeddingPolicy::enabled_with_profile("local_code"),
    )
    .expect("write semantic policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("[semantic_clones]"));
    assert!(content.contains("embedding_mode = \"semantic_aware_once\""));
    assert!(content.contains("[semantic_clones.inference]"));
    assert!(content.contains("code_embeddings = \"local_code\""));
    assert!(content.contains("summary_embeddings = \"local_code\""));
}

#[test]
fn enabled_embedding_profile_preserves_only_existing_summary_embedding_choice() {
    let existing_without_summary_embeddings = RepoSemanticEmbeddingPolicy {
        present: true,
        summary_mode: Some(SemanticSummaryMode::Auto),
        embedding_mode: Some(SemanticCloneEmbeddingMode::Off),
        inference: SemanticClonesInferenceBindings {
            summary_generation: Some("summary_local".to_string()),
            code_embeddings: None,
            summary_embeddings: None,
        },
    };
    let policy = RepoSemanticEmbeddingPolicy::enabled_with_profile_preserving_summaries(
        "local_code",
        &existing_without_summary_embeddings,
    );
    assert_eq!(
        policy.inference.code_embeddings.as_deref(),
        Some("local_code")
    );
    assert_eq!(
        policy.inference.summary_generation.as_deref(),
        Some("summary_local")
    );
    assert_eq!(policy.inference.summary_embeddings, None);

    let mut existing_with_summary_embeddings = existing_without_summary_embeddings.clone();
    existing_with_summary_embeddings
        .inference
        .summary_embeddings = Some("old_summary".to_string());
    let policy = RepoSemanticEmbeddingPolicy::enabled_with_profile_preserving_summaries(
        "platform_code",
        &existing_with_summary_embeddings,
    );
    assert_eq!(
        policy.inference.summary_embeddings.as_deref(),
        Some("platform_code")
    );
}

#[test]
fn set_repo_semantic_embedding_policy_removes_bindings_when_disabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);
    set_repo_semantic_embedding_policy(
        &path,
        &RepoSemanticEmbeddingPolicy::enabled_with_profile("local_code"),
    )
    .expect("write enabled semantic policy");

    set_repo_semantic_embedding_policy(&path, &RepoSemanticEmbeddingPolicy::disabled())
        .expect("write disabled semantic policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("embedding_mode = \"off\""));
    assert!(!content.contains("code_embeddings"));
    assert!(!content.contains("summary_embeddings"));
}

#[test]
fn set_repo_semantic_embedding_policy_persists_summary_mode_and_generation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);

    set_repo_semantic_embedding_policy(
        &path,
        &RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(SemanticSummaryMode::Auto),
            embedding_mode: Some(SemanticCloneEmbeddingMode::SemanticAwareOnce),
            inference: SemanticClonesInferenceBindings {
                summary_generation: Some("summary_local".to_string()),
                code_embeddings: Some("code_local".to_string()),
                summary_embeddings: None,
            },
        },
    )
    .expect("write semantic policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("summary_mode = \"auto\""));
    assert!(content.contains("embedding_mode = \"semantic_aware_once\""));
    assert!(content.contains("summary_generation = \"summary_local\""));
    assert!(content.contains("code_embeddings = \"code_local\""));
    assert!(!content.contains("summary_embeddings = "));
}

#[test]
fn set_repo_semantic_embedding_policy_can_disable_summaries_without_disabling_embeddings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);

    set_repo_semantic_embedding_policy(
        &path,
        &RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(SemanticSummaryMode::Off),
            embedding_mode: Some(SemanticCloneEmbeddingMode::SemanticAwareOnce),
            inference: SemanticClonesInferenceBindings {
                summary_generation: None,
                code_embeddings: Some("code_local".to_string()),
                summary_embeddings: None,
            },
        },
    )
    .expect("write semantic policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("summary_mode = \"off\""));
    assert!(content.contains("embedding_mode = \"semantic_aware_once\""));
    assert!(content.contains("code_embeddings = \"code_local\""));
    assert!(!content.contains("summary_generation = "));
    assert!(!content.contains("summary_embeddings = "));
}

#[test]
fn set_repo_semantic_embedding_policy_removes_summary_mode_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);
    fs::write(
        &path,
        r#"
[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
summary_generation = "summary_local"
code_embeddings = "old_code"
"#,
    )
    .expect("write initial policy");

    set_repo_semantic_embedding_policy(
        &path,
        &RepoSemanticEmbeddingPolicy::enabled_with_profile("new_code"),
    )
    .expect("write embedding-only policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(!content.contains("summary_mode = "));
    assert!(!content.contains("summary_generation = "));
    assert!(content.contains("embedding_mode = \"semantic_aware_once\""));
    assert!(content.contains("code_embeddings = \"new_code\""));
    assert!(content.contains("summary_embeddings = \"new_code\""));
}

#[test]
fn devql_producer_settings_reject_non_boolean_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[devql]
sync_enabled = "yes"
ingest_enabled = false
"#,
    )
    .expect("write local repo policy");

    let err = devql_producer_settings(dir.path()).expect_err("non-bool flag should fail");

    assert!(format!("{err:#}").contains("`[devql].sync_enabled` must be a boolean"));
}

#[test]
fn repo_semantic_embedding_policy_rejects_non_table_inference() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"
[semantic_clones]
embedding_mode = "semantic_aware_once"
inference = "local_code"
"#,
    )
    .expect("write local repo policy");

    let err =
        repo_semantic_embedding_policy(dir.path()).expect_err("non-table inference should fail");

    assert!(format!("{err:#}").contains("`[semantic_clones].inference` must be a table"));
}

#[test]
fn devql_guidance_enabled_rejects_non_boolean_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_FILE_NAME),
        r#"
[capture]
enabled = true

[agents]
supported = ["codex"]
devql_guidance_enabled = "sometimes"
"#,
    )
    .expect("write repo policy");

    let err = devql_guidance_enabled(dir.path()).expect_err("non-bool flag should fail");
    assert!(format!("{err:#}").contains("`[agents].devql_guidance_enabled` must be a boolean"));
}

#[test]
fn devql_guidance_enabled_ignores_legacy_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(REPO_POLICY_FILE_NAME),
        r#"
[capture]
enabled = true

[agents]
supported = ["codex"]
devql_skill_enabled = false
"#,
    )
    .expect("write repo policy");

    assert!(devql_guidance_enabled(dir.path()).expect("load devql guidance flag"));
}

#[test]
fn write_project_bootstrap_settings_persists_devql_guidance_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(REPO_POLICY_LOCAL_FILE_NAME);

    write_project_bootstrap_settings_with_daemon_binding_and_devql_guidance(
        &path,
        "manual-commit",
        &["codex".to_string()],
        None,
        false,
    )
    .expect("write repo policy");

    let content = fs::read_to_string(path).expect("read repo policy");
    assert!(content.contains("devql_guidance_enabled = false"));
}
