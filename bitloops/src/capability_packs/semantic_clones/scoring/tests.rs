use super::*;

fn sample_input(symbol_id: &str, name: &str) -> SymbolCloneCandidateInput {
    SymbolCloneCandidateInput {
        repo_id: "repo-1".to_string(),
        symbol_id: symbol_id.to_string(),
        artefact_id: format!("artefact-{symbol_id}"),
        path: "src/services/orders.ts".to_string(),
        canonical_kind: "function".to_string(),
        symbol_fqn: format!("src/services/orders.ts::{name}"),
        summary: format!("Function {name}."),
        normalized_name: name.to_string(),
        normalized_signature: Some(format!("function {name}(id: string, opts: number)")),
        identifier_tokens: vec!["order".to_string(), "fetch".to_string(), "id".to_string()],
        normalized_body_tokens: vec![
            "return".to_string(),
            "db".to_string(),
            "order".to_string(),
            "fetch".to_string(),
        ],
        parent_kind: Some("module".to_string()),
        context_tokens: vec!["services".to_string(), "orders".to_string()],
        embedding_setup: EmbeddingSetup::new(
            "local_fastembed",
            "jinaai/jina-embeddings-v2-base-code",
            3,
        ),
        embedding: vec![0.9, 0.1, 0.0],
        call_targets: vec!["db.fetchOrder".to_string()],
        dependency_targets: vec!["references:order_repository::entity".to_string()],
        churn_count: 1,
    }
}

#[test]
fn clone_scoring_options_clamp_neighbors() {
    assert_eq!(
        CloneScoringOptions::default().ann_neighbors,
        DEFAULT_ANN_NEIGHBORS
    );
    assert!(CloneScoringOptions::default().ann_enabled);
    assert_eq!(CloneScoringOptions::new(0).ann_neighbors, MIN_ANN_NEIGHBORS);
    assert_eq!(
        CloneScoringOptions::new(MAX_ANN_NEIGHBORS + 10).ann_neighbors,
        MAX_ANN_NEIGHBORS
    );
    assert_eq!(
        CloneScoringOptions::from_i64_clamped(-10).ann_neighbors,
        MIN_ANN_NEIGHBORS
    );
    assert!(
        !CloneScoringOptions::new(5)
            .with_ann_enabled(false)
            .ann_enabled
    );
}

#[test]
fn clone_scoring_options_disable_ann_env_turns_off_prefilter() {
    crate::test_support::process_state::with_env_var(DISABLE_ANN_ENV, Some("true"), || {
        let options = CloneScoringOptions::new(5).apply_env_overrides();
        assert!(!options.ann_enabled);
    });
}

#[test]
fn clone_scoring_options_disable_ann_env_is_case_insensitive() {
    crate::test_support::process_state::with_env_var(DISABLE_ANN_ENV, Some("YeS"), || {
        assert!(ann_disabled_from_env());
    });
    crate::test_support::process_state::with_env_var(DISABLE_ANN_ENV, Some("0"), || {
        assert!(!ann_disabled_from_env());
    });
}

#[test]
fn build_symbol_clone_edges_marks_exact_duplicates() {
    let source = sample_input("source", "fetch_order");
    let mut target = sample_input("target", "fetch_order");
    target.path = "src/services/order_copies.ts".to_string();
    target.symbol_fqn = "src/services/order_copies.ts::fetch_order".to_string();

    let result = build_symbol_clone_edges(&[source, target]);

    assert_eq!(result.edges.len(), 2);
    assert!(
        result
            .edges
            .iter()
            .all(|edge| edge.relation_kind == RELATION_KIND_EXACT_DUPLICATE)
    );
}

#[test]
fn build_symbol_clone_edges_marks_diverged_implementations() {
    let mut source = sample_input("source", "validate_order_checkout");
    source.embedding = vec![0.9, 0.2, 0.0];
    source.call_targets = vec!["rules.checkout".to_string()];
    source.normalized_body_tokens = vec!["validate".to_string(), "checkout".to_string()];

    let mut target = sample_input("target", "validate_order_draft");
    target.embedding = vec![0.7, 0.3, 0.0];
    target.call_targets = vec!["rules.draft".to_string()];
    target.normalized_body_tokens = vec!["validate".to_string(), "draft".to_string()];

    let result = build_symbol_clone_edges(&[source, target]);

    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.relation_kind == RELATION_KIND_DIVERGED_IMPLEMENTATION)
    );
}

#[test]
fn build_symbol_clone_edges_skips_cross_kind_matches() {
    let source = sample_input("source", "get_root_handler");

    let mut target = sample_input("target", "root_ts");
    target.canonical_kind = "file".to_string();
    target.symbol_fqn = "src/services/orders.ts".to_string();
    target.normalized_signature = None;

    let result = build_symbol_clone_edges(&[source, target]);

    assert!(result.edges.is_empty());
}

#[test]
fn build_symbol_clone_edges_skips_import_candidates() {
    let mut source = sample_input("source", "import_src");
    source.canonical_kind = "import".to_string();
    source.normalized_signature = Some("import foo from 'bar'".to_string());
    source.identifier_tokens = vec!["foo".to_string(), "bar".to_string(), "import".to_string()];
    source.normalized_body_tokens = vec!["import".to_string(), "foo".to_string()];

    let mut target = sample_input("target", "import_target");
    target.canonical_kind = "import".to_string();
    target.normalized_signature = Some("import baz from 'bar'".to_string());
    target.identifier_tokens = vec!["baz".to_string(), "bar".to_string(), "import".to_string()];
    target.normalized_body_tokens = vec!["import".to_string(), "baz".to_string()];

    let result = build_symbol_clone_edges(&[source, target]);

    assert!(result.edges.is_empty());
    assert_eq!(result.sources_considered, 0);
}

#[test]
fn build_symbol_clone_edges_labels_preferred_local_patterns() {
    let mut source = sample_input("source", "render_invoice_document");
    source.embedding = vec![0.8, 0.2, 0.1];

    let mut target = sample_input("target", "create_invoice_pdf");
    target.embedding = vec![0.82, 0.18, 0.1];
    target.churn_count = 1;
    target.path = "src/billing/invoice.ts".to_string();

    let result = build_symbol_clone_edges(&[source, target]);
    let labels = result.edges[0]
        .explanation_json
        .get("labels")
        .and_then(Value::as_array);
    assert!(labels.is_some());
}

#[test]
fn build_symbol_clone_edges_marks_contextual_neighbors_when_locality_dominates() {
    let mut source = sample_input("source", "execute");
    source.canonical_kind = "method".to_string();
    source.parent_kind = Some("class_declaration".to_string());
    source.path = "src/handlers/change-path.ts".to_string();
    source.symbol_fqn =
        "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::execute".to_string();
    source.summary = "Method execute. Applies the path change workflow.".to_string();
    source.identifier_tokens = vec![
        "change".to_string(),
        "path".to_string(),
        "code".to_string(),
        "file".to_string(),
    ];
    source.normalized_body_tokens = vec![
        "load".to_string(),
        "validate".to_string(),
        "rename".to_string(),
    ];
    source.call_targets = vec!["repo.loadFile".to_string(), "domain.renamePath".to_string()];

    let mut target = sample_input("target", "command");
    target.canonical_kind = "method".to_string();
    target.parent_kind = Some("class_declaration".to_string());
    target.path = source.path.clone();
    target.symbol_fqn =
        "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::command".to_string();
    target.summary = "Method command. Returns the command payload.".to_string();
    target.identifier_tokens = vec![
        "change".to_string(),
        "path".to_string(),
        "command".to_string(),
        "file".to_string(),
    ];
    target.normalized_body_tokens = vec![
        "return".to_string(),
        "command".to_string(),
        "payload".to_string(),
    ];
    target.call_targets = vec!["factory.buildCommand".to_string()];

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("contextual neighbor edge");

    assert_eq!(edge.relation_kind, RELATION_KIND_WEAK_CLONE_CANDIDATE);
    assert!(edge.score < 0.75);
    assert_eq!(
        edge.explanation_json["confidence"]["confidence_band"],
        Value::String("weak".to_string())
    );
    assert!(edge.explanation_json["evidence"]["bias_warning"].as_str() == Some("same_file_bias"));
}

#[test]
fn build_symbol_clone_edges_keeps_same_file_clone_confidence_when_impl_is_strong() {
    let mut source = sample_input("source", "apply_path_change");
    source.canonical_kind = "method".to_string();
    source.parent_kind = Some("class_declaration".to_string());
    source.path = "src/handlers/change-path.ts".to_string();
    source.symbol_fqn =
        "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::apply_path_change"
            .to_string();
    source.summary = "Method apply path change. Applies the path change to the file.".to_string();
    source.identifier_tokens = vec![
        "apply".to_string(),
        "path".to_string(),
        "change".to_string(),
        "file".to_string(),
    ];
    source.normalized_body_tokens = vec![
        "load".to_string(),
        "validate".to_string(),
        "rename".to_string(),
        "persist".to_string(),
    ];
    source.call_targets = vec![
        "repo.loadFile".to_string(),
        "domain.renamePath".to_string(),
        "repo.persistFile".to_string(),
    ];

    let mut target = sample_input("target", "apply_path_change_for_move");
    target.canonical_kind = "method".to_string();
    target.parent_kind = Some("class_declaration".to_string());
    target.path = source.path.clone();
    target.symbol_fqn =
        "src/handlers/change-path.ts::ChangePathOfCodeFileCommandHandler::apply_path_change_for_move"
            .to_string();
    target.summary =
        "Method apply path change for move. Applies the path change and persists it.".to_string();
    target.identifier_tokens = vec![
        "apply".to_string(),
        "path".to_string(),
        "change".to_string(),
        "move".to_string(),
    ];
    target.normalized_body_tokens = vec![
        "load".to_string(),
        "validate".to_string(),
        "rename".to_string(),
        "persist".to_string(),
        "emit".to_string(),
    ];
    target.call_targets = vec![
        "repo.loadFile".to_string(),
        "domain.renamePath".to_string(),
        "repo.persistFile".to_string(),
    ];

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("same-file strong clone edge");

    assert_ne!(edge.relation_kind, RELATION_KIND_WEAK_CLONE_CANDIDATE);
    assert!(
        edge.explanation_json["confidence"]["clone_confidence"]
            .as_f64()
            .expect("clone confidence")
            >= CLONE_CONFIDENCE_MEDIUM_THRESHOLD as f64
    );
    assert!(edge.explanation_json["evidence"]["bias_warning"].is_null());
}

#[test]
fn build_symbol_clone_edges_exposes_dependency_overlap() {
    let mut source = sample_input("source", "validate_path");
    source.call_targets = vec!["repo.loadFile".to_string()];
    source.dependency_targets = vec![
        "references:path_service::path".to_string(),
        "implements:path_validator".to_string(),
    ];

    let mut target = sample_input("target", "validate_moved_path");
    target.call_targets = vec!["repo.loadMovedFile".to_string()];
    target.dependency_targets = vec![
        "references:path_service::path".to_string(),
        "implements:path_validator".to_string(),
    ];

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("dependency-aware clone edge");

    assert!(
        edge.explanation_json["scores"]["dependency_overlap"]
            .as_f64()
            .expect("dependency overlap")
            > 0.0
    );
    assert_eq!(
        edge.explanation_json["evidence"]["shared_signals"]["dependency_targets"][0],
        Value::String("implements:path_validator".to_string())
    );
}

#[test]
fn semantic_similarity_requires_matching_provider_and_model() {
    let source = sample_input("source", "fetch_order");
    let mut target = sample_input("target", "fetch_order_copy");
    target.embedding_setup =
        EmbeddingSetup::new("voyage", "voyage-code-3", target.embedding_setup.dimension);
    assert_eq!(semantic_similarity(&source, &target), 0.0);

    let mut target = sample_input("target2", "fetch_order_copy_2");
    target.embedding_setup = EmbeddingSetup::new(
        target.embedding_setup.provider.clone(),
        "other-model",
        target.embedding_setup.dimension,
    );
    assert_eq!(semantic_similarity(&source, &target), 0.0);
}

#[test]
fn semantic_similarity_requires_matching_dimension() {
    let source = sample_input("source", "fetch_order");
    let mut target = sample_input("target", "fetch_order_copy");
    target.embedding_setup = EmbeddingSetup::new(
        target.embedding_setup.provider.clone(),
        target.embedding_setup.model.clone(),
        6,
    );

    assert_eq!(semantic_similarity(&source, &target), 0.0);
}

#[test]
fn build_symbol_clone_edges_for_source_respects_ann_neighbors_prefilter() {
    let source = sample_input("source", "alpha");

    let mut top = sample_input("top", "alpha_top");
    top.path = "src/services/top.ts".to_string();
    top.symbol_fqn = "src/services/top.ts::alpha_top".to_string();
    top.embedding = vec![0.91, 0.09, 0.0];

    let mut other = sample_input("other", "alpha_other");
    other.path = "src/services/other.ts".to_string();
    other.symbol_fqn = "src/services/other.ts::alpha_other".to_string();
    other.embedding = vec![0.2, 0.8, 0.0];

    let result = build_symbol_clone_edges_for_source_with_options(
        &[source, top, other],
        "source",
        CloneScoringOptions::new(1),
    );

    let source_edges = result
        .edges
        .iter()
        .filter(|edge| edge.source_symbol_id == "source")
        .collect::<Vec<_>>();
    assert!(source_edges.len() <= 1);
}

#[test]
fn low_ann_neighbors_keeps_exact_duplicate_recall_via_duplicate_bucket() {
    let mut source = sample_input("source", "fetch_order");
    source.embedding = vec![1.0, 0.0, 0.0];

    let mut nearest_non_duplicate = sample_input("nearest", "fetch_order_nearest");
    nearest_non_duplicate.path = "src/services/nearest.ts".to_string();
    nearest_non_duplicate.symbol_fqn = "src/services/nearest.ts::fetch_order_nearest".to_string();
    nearest_non_duplicate.normalized_name = "fetch_order_nearest".to_string();
    nearest_non_duplicate.normalized_body_tokens = vec!["fetch".to_string(), "nearest".to_string()];
    nearest_non_duplicate.normalized_signature =
        Some("function fetch_order_nearest(id: string)".to_string());
    nearest_non_duplicate.embedding = vec![0.99, 0.01, 0.0];

    let mut exact_duplicate = sample_input("duplicate", "fetch_order");
    exact_duplicate.path = "src/services/duplicate.ts".to_string();
    exact_duplicate.symbol_fqn = "src/services/duplicate.ts::fetch_order".to_string();
    // Make the duplicate semantically far so ANN(1) would likely miss it without backfill.
    exact_duplicate.embedding = vec![0.0, 1.0, 0.0];

    let result = build_symbol_clone_edges_for_source_with_options(
        &[source, nearest_non_duplicate, exact_duplicate],
        "source",
        CloneScoringOptions::new(1),
    );

    assert!(result.edges.iter().any(|edge| {
        edge.source_symbol_id == "source"
            && edge.target_symbol_id == "duplicate"
            && edge.relation_kind == RELATION_KIND_EXACT_DUPLICATE
    }));
}

#[test]
fn disabling_ann_keeps_exact_duplicate_recall() {
    let mut source = sample_input("source", "fetch_order");
    source.embedding = vec![1.0, 0.0, 0.0];

    let mut nearest_non_duplicate = sample_input("nearest", "fetch_order_nearest");
    nearest_non_duplicate.path = "src/services/nearest.ts".to_string();
    nearest_non_duplicate.symbol_fqn = "src/services/nearest.ts::fetch_order_nearest".to_string();
    nearest_non_duplicate.normalized_name = "fetch_order_nearest".to_string();
    nearest_non_duplicate.normalized_body_tokens = vec!["fetch".to_string(), "nearest".to_string()];
    nearest_non_duplicate.normalized_signature =
        Some("function fetch_order_nearest(id: string)".to_string());
    nearest_non_duplicate.embedding = vec![0.99, 0.01, 0.0];

    let mut exact_duplicate = sample_input("duplicate", "fetch_order");
    exact_duplicate.path = "src/services/duplicate.ts".to_string();
    exact_duplicate.symbol_fqn = "src/services/duplicate.ts::fetch_order".to_string();
    exact_duplicate.embedding = vec![0.0, 1.0, 0.0];

    let result = build_symbol_clone_edges_for_source_with_options(
        &[source, nearest_non_duplicate, exact_duplicate],
        "source",
        CloneScoringOptions::new(1).with_ann_enabled(false),
    );

    assert!(result.edges.iter().any(|edge| {
        edge.source_symbol_id == "source"
            && edge.target_symbol_id == "duplicate"
            && edge.relation_kind == RELATION_KIND_EXACT_DUPLICATE
    }));
}
