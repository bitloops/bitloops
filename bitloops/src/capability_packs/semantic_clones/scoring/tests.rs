use super::*;

const TEST_EMBEDDINGS_DRIVER: &str = crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER;
const TEST_EMBEDDINGS_MODEL: &str = "bge-m3";

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
        embedding_setup: EmbeddingSetup::new(TEST_EMBEDDINGS_DRIVER, TEST_EMBEDDINGS_MODEL, 3),
        embedding: vec![0.9, 0.1, 0.0],
        summary_embedding_setup: None,
        summary_embedding: Vec::new(),
        call_targets: vec!["db.fetchOrder".to_string()],
        dependency_targets: vec!["references:order_repository::entity".to_string()],
        churn_count: 1,
    }
}

fn attach_summary_embedding(
    input: &mut SymbolCloneCandidateInput,
    setup: &EmbeddingSetup,
    embedding: Vec<f32>,
) {
    input.summary_embedding_setup = Some(setup.clone());
    input.summary_embedding = embedding;
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
    let options = CloneScoringOptions::new(5).apply_ann_override_raw(Some("true"));
    assert!(!options.ann_enabled);
}

#[test]
fn clone_scoring_options_disable_ann_env_is_case_insensitive() {
    assert!(ann_disabled_from_raw("YeS"));
    assert!(!ann_disabled_from_raw("0"));
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
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "validate_order_checkout");
    source.embedding = vec![1.0, 0.0, 0.0];
    source.call_targets = vec!["rules.checkout".to_string()];
    source.normalized_body_tokens = vec!["validate".to_string(), "checkout".to_string()];
    source.summary = "Function validate order checkout. Validates checkout rules.".to_string();
    attach_summary_embedding(&mut source, &summary_setup, vec![1.0, 0.0, 0.0]);

    let mut target = sample_input("target", "validate_order_draft");
    target.embedding = vec![0.99, 0.01, 0.0];
    target.call_targets = vec!["rules.draft".to_string()];
    target.normalized_body_tokens = vec!["validate".to_string(), "draft".to_string()];
    target.summary = "Function validate order draft. Validates draft rules.".to_string();
    attach_summary_embedding(&mut target, &summary_setup, vec![-1.0, 0.0, 0.0]);

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
fn build_symbol_clone_edges_uses_text_fallback_without_forcing_reuse_drift() {
    let mut source = sample_input("source", "create_invoice_pdf");
    source.path = "src/pdf.ts".to_string();
    source.symbol_fqn = "src/pdf.ts::create_invoice_pdf".to_string();
    source.summary =
        "Function create invoice pdf. Generates an invoice PDF for an order.".to_string();
    source.identifier_tokens = vec![
        "create".to_string(),
        "invoice".to_string(),
        "pdf".to_string(),
        "order".to_string(),
        "locale".to_string(),
    ];
    source.normalized_body_tokens = vec![
        "generate".to_string(),
        "invoice".to_string(),
        "pdf".to_string(),
        "order".to_string(),
        "locale".to_string(),
    ];
    source.context_tokens = vec!["src".to_string(), "pdf".to_string(), "invoice".to_string()];
    source.embedding = vec![0.91, 0.09, 0.0];

    let mut target = sample_input("target", "render_invoice_document");
    target.path = "src/render.ts".to_string();
    target.symbol_fqn = "src/render.ts::render_invoice_document".to_string();
    target.summary =
        "Function render invoice document. Renders the invoice document for an order.".to_string();
    target.identifier_tokens = vec![
        "render".to_string(),
        "invoice".to_string(),
        "document".to_string(),
        "order".to_string(),
        "locale".to_string(),
    ];
    target.normalized_body_tokens = vec![
        "render".to_string(),
        "invoice".to_string(),
        "document".to_string(),
        "order".to_string(),
        "locale".to_string(),
    ];
    target.context_tokens = vec![
        "src".to_string(),
        "render".to_string(),
        "invoice".to_string(),
    ];
    target.embedding = vec![0.89, 0.11, 0.0];

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("text-fallback similar implementation edge");

    assert_eq!(edge.relation_kind, RELATION_KIND_SIMILAR_IMPLEMENTATION);
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["summary_signal_source"],
        Value::String("text_fallback".to_string())
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["interpretation"],
        Value::String("same_behaviour_similar_implementation".to_string())
    );
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
fn build_symbol_clone_edges_for_source_unions_code_and_summary_ann_neighbors() {
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "normalize_checkout");
    source.summary_embedding_setup = Some(summary_setup.clone());
    source.summary_embedding = vec![1.0, 0.0, 0.0];

    let mut code_neighbor = sample_input("code", "normalize_checkout_fast");
    code_neighbor.path = "src/services/code_neighbor.ts".to_string();
    code_neighbor.symbol_fqn = "src/services/code_neighbor.ts::normalize_checkout_fast".to_string();
    code_neighbor.embedding = vec![0.99, 0.01, 0.0];
    code_neighbor.summary_embedding_setup = Some(summary_setup.clone());
    code_neighbor.summary_embedding = vec![0.0, 1.0, 0.0];

    let mut summary_neighbor = sample_input("summary", "normalize_checkout_alt");
    summary_neighbor.path = "src/services/summary_neighbor.ts".to_string();
    summary_neighbor.symbol_fqn =
        "src/services/summary_neighbor.ts::normalize_checkout_alt".to_string();
    summary_neighbor.embedding = vec![0.0, 1.0, 0.0];
    summary_neighbor.summary_embedding_setup = Some(summary_setup);
    summary_neighbor.summary_embedding = vec![0.99, 0.01, 0.0];

    let result = build_symbol_clone_edges_for_source_with_options(
        &[source, code_neighbor, summary_neighbor],
        "source",
        CloneScoringOptions::new(1),
    );

    let source_targets = result
        .edges
        .iter()
        .filter(|edge| edge.source_symbol_id == "source")
        .map(|edge| edge.target_symbol_id.as_str())
        .collect::<Vec<_>>();
    assert!(source_targets.contains(&"code"));
    assert!(source_targets.contains(&"summary"));
}

#[test]
fn degenerate_embedding_groups_fall_back_to_full_group_scoring() {
    let mut source = sample_input("source", "render_invoice");
    source.path = "src/render/render_invoice.ts".to_string();
    source.symbol_fqn = "src/render/render_invoice.ts::render_invoice".to_string();
    source.normalized_name = "render_invoice".to_string();
    source.normalized_signature =
        Some("function render_invoice(order_id: string, locale: string)".to_string());
    source.identifier_tokens = vec![
        "render".to_string(),
        "invoice".to_string(),
        "order".to_string(),
    ];
    source.normalized_body_tokens = vec![
        "return".to_string(),
        "render".to_string(),
        "header".to_string(),
        "body".to_string(),
        "join".to_string(),
    ];
    source.context_tokens = vec!["render".to_string(), "invoice".to_string()];
    source.call_targets = vec![
        "createRenderHeader".to_string(),
        "createRenderBody".to_string(),
    ];
    source.dependency_targets = vec!["references:common_snapshot_utils".to_string()];
    source.embedding = vec![0.1, 0.2, 0.3];

    let mut target = sample_input("target", "render_invoice_document");
    target.path = "src/render/render_invoice_document.ts".to_string();
    target.symbol_fqn =
        "src/render/render_invoice_document.ts::render_invoice_document".to_string();
    target.normalized_name = "render_invoice_document".to_string();
    target.normalized_signature =
        Some("function render_invoice_document(order_id: string, locale: string)".to_string());
    target.identifier_tokens = vec![
        "render".to_string(),
        "invoice".to_string(),
        "document".to_string(),
    ];
    target.normalized_body_tokens = source.normalized_body_tokens.clone();
    target.context_tokens = source.context_tokens.clone();
    target.call_targets = source.call_targets.clone();
    target.dependency_targets = source.dependency_targets.clone();
    target.embedding = source.embedding.clone();

    let mut candidates = vec![source.clone()];
    for idx in 0..6 {
        let mut distractor = sample_input(&format!("distractor-{idx}"), "archive_customer");
        distractor.path = format!("src/archive/distractor_{idx}.ts");
        distractor.symbol_fqn = format!("src/archive/distractor_{idx}.ts::archive_customer");
        distractor.normalized_name = format!("archive_customer_{idx}");
        distractor.normalized_signature =
            Some(format!("function archive_customer_{idx}(customer_id: string)"));
        distractor.identifier_tokens = vec![
            "archive".to_string(),
            "customer".to_string(),
            idx.to_string(),
        ];
        distractor.normalized_body_tokens = vec![
            "archive".to_string(),
            "customer".to_string(),
            "record".to_string(),
        ];
        distractor.context_tokens = vec!["archive".to_string(), "customer".to_string()];
        distractor.call_targets = vec!["archiveCustomer".to_string()];
        distractor.dependency_targets = vec!["references:archive_repository".to_string()];
        distractor.embedding = source.embedding.clone();
        candidates.push(distractor);
    }
    candidates.push(target);

    let result = build_symbol_clone_edges_for_source_with_options(
        &candidates,
        "source",
        CloneScoringOptions::new(1),
    );

    assert!(result.edges.iter().any(|edge| {
        edge.source_symbol_id == "source" && edge.target_symbol_id == "target"
    }));
}

#[test]
fn build_symbol_clone_edges_exposes_multi_view_similarity_pattern() {
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "normalize_checkout");
    attach_summary_embedding(&mut source, &summary_setup, vec![1.0, 0.0, 0.0]);

    let mut target = sample_input("target", "normalize_checkout_copy");
    target.path = "src/services/target.ts".to_string();
    target.symbol_fqn = "src/services/target.ts::normalize_checkout_copy".to_string();
    target.summary = "Function archive draft invoice.".to_string();
    target.embedding = vec![0.99, 0.01, 0.0];
    attach_summary_embedding(&mut target, &summary_setup, vec![-1.0, 0.0, 0.0]);

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("multi-view clone edge");

    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["match_pattern"],
        Value::String("high_low".to_string())
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["interpretation"],
        Value::String("implementation_reuse_drift".to_string())
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["primary_driver"],
        Value::String("code".to_string())
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["summary_signal_source"],
        Value::String("embedding".to_string())
    );
    assert!(
        edge.explanation_json["scores"]["code_embedding"]
            .as_f64()
            .expect("code embedding score")
            > edge.explanation_json["scores"]["summary_embedding"]
                .as_f64()
                .expect("summary embedding score")
    );
}

#[test]
fn build_symbol_clone_edges_uses_summary_embedding_over_text_overlap_when_available() {
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "normalize_checkout");
    source.embedding = vec![1.0, 0.0, 0.0];
    source.summary = "Function archive draft invoice.".to_string();
    attach_summary_embedding(&mut source, &summary_setup, vec![1.0, 0.0, 0.0]);

    let mut target = sample_input("target", "normalize_checkout_copy");
    target.path = "src/services/target.ts".to_string();
    target.symbol_fqn = "src/services/target.ts::normalize_checkout_copy".to_string();
    target.embedding = vec![0.99, 0.01, 0.0];
    target.summary = source.summary.clone();
    attach_summary_embedding(&mut target, &summary_setup, vec![-1.0, 0.0, 0.0]);

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("vector-first clone edge");

    assert_eq!(
        edge.explanation_json["scores"]["summary_similarity"],
        Value::from(0.0)
    );
    assert_eq!(
        edge.explanation_json["scores"]["summary_text_similarity"],
        Value::from(1.0)
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["summary_signal_source"],
        Value::String("embedding".to_string())
    );
}

#[test]
fn build_symbol_clone_edges_marks_summary_driven_behaviour_matches_as_shared_logic() {
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "normalize_checkout");
    source.embedding = vec![1.0, 0.0, 0.0];
    source.summary = "Validates a checkout request before final submission.".to_string();
    attach_summary_embedding(&mut source, &summary_setup, vec![1.0, 0.0, 0.0]);

    let mut target = sample_input("target", "validate_checkout_workflow");
    target.path = "src/services/summary_neighbor.ts".to_string();
    target.symbol_fqn = "src/services/summary_neighbor.ts::validate_checkout_workflow".to_string();
    target.embedding = vec![-1.0, 0.0, 0.0];
    target.summary = "Validates a checkout request before final submission.".to_string();
    attach_summary_embedding(&mut target, &summary_setup, vec![0.99, 0.01, 0.0]);

    let result = build_symbol_clone_edges(&[source, target]);
    let edge = result
        .edges
        .iter()
        .find(|edge| edge.target_symbol_id == "target")
        .expect("summary-driven clone edge");

    assert_eq!(edge.relation_kind, RELATION_KIND_SHARED_LOGIC_CANDIDATE);
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["interpretation"],
        Value::String("same_behaviour_different_implementation".to_string())
    );
    assert_eq!(
        edge.explanation_json["evidence"]["semantic_views"]["primary_driver"],
        Value::String("summary".to_string())
    );
}

#[test]
fn low_code_low_summary_pairs_are_unrelated_and_do_not_emit_edges() {
    let summary_setup = EmbeddingSetup::new("openai", "text-embedding-3-large", 3);

    let mut source = sample_input("source", "normalize_checkout");
    source.embedding = vec![1.0, 0.0, 0.0];
    source.summary = "Validates a checkout request before final submission.".to_string();
    attach_summary_embedding(&mut source, &summary_setup, vec![1.0, 0.0, 0.0]);

    let mut target = sample_input("target", "archive_customer_profile");
    target.path = "src/archive/customer.ts".to_string();
    target.symbol_fqn = "src/archive/customer.ts::archive_customer_profile".to_string();
    target.normalized_name = "archive_customer_profile".to_string();
    target.normalized_signature =
        Some("function archive_customer_profile(customerId: string)".to_string());
    target.identifier_tokens = vec![
        "archive".to_string(),
        "customer".to_string(),
        "profile".to_string(),
    ];
    target.normalized_body_tokens = vec![
        "archive".to_string(),
        "customer".to_string(),
        "profile".to_string(),
    ];
    target.context_tokens = vec!["archive".to_string(), "customer".to_string()];
    target.call_targets = vec!["archiveRepo.persist".to_string()];
    target.dependency_targets = vec!["references:archive_repository::entity".to_string()];
    target.embedding = vec![-1.0, 0.0, 0.0];
    target.summary = "Archives a customer profile for retention.".to_string();
    attach_summary_embedding(&mut target, &summary_setup, vec![-1.0, 0.0, 0.0]);

    let lexical = lexical_signals(&source, &target);
    let structural = structural_signals(&source, &target, lexical.name_match);
    let derived = derived_clone_signals(&source, &target, 0.0, Some(0.0), &lexical, &structural);

    assert_eq!(derived.interpretation, SemanticInterpretation::Unrelated);
    assert!(build_symbol_clone_edges(&[source, target]).edges.is_empty());
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
