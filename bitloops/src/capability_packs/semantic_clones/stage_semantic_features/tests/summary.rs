use super::super::summary::{SemanticSummarySnapshot, ensure_required_llm_summary_output};
use super::support::{StrictNoopSummaryProvider, sample_semantic_input};
use crate::capability_packs::semantic_clones::features as semantic;

fn semantic_rows(
    llm_summary: Option<&str>,
    summary: &str,
    confidence: Option<f32>,
    source_model: Option<&str>,
) -> semantic::SemanticFeatureRows {
    let input = semantic::SemanticFeatureInput {
        canonical_kind: "file".to_string(),
        language_kind: "source_file".to_string(),
        symbol_fqn: "src/lib.rs".to_string(),
        name: "lib".to_string(),
        signature: None,
        modifiers: Vec::new(),
        body: "fn main() {}".to_string(),
        docstring: None,
        parent_kind: None,
        dependency_signals: Vec::new(),
        ..sample_semantic_input("artefact-1", "blob-1")
    };

    semantic::SemanticFeatureRows {
        semantics: semantic::SymbolSemanticsRow {
            artefact_id: "artefact-1".to_string(),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            docstring_summary: None,
            llm_summary: llm_summary.map(str::to_string),
            template_summary: "Defines the rust source file.".to_string(),
            summary: summary.to_string(),
            confidence,
            source_model: source_model.map(str::to_string),
        },
        features: semantic::build_semantic_feature_rows(
            &input,
            &semantic::NoopSemanticSummaryProvider,
        )
        .features,
        semantic_features_input_hash: "hash-1".to_string(),
    }
}

#[test]
fn semantic_summary_snapshot_marks_llm_enrichment_from_summary_or_model() {
    assert!(
        SemanticSummarySnapshot {
            semantic_features_input_hash: "hash-1".to_string(),
            summary: "Function load user.".to_string(),
            llm_summary: Some("Loads a user by id.".to_string()),
            source_model: None,
        }
        .is_llm_enriched()
    );

    assert!(
        SemanticSummarySnapshot {
            semantic_features_input_hash: "hash-1".to_string(),
            summary: "Function load user.".to_string(),
            llm_summary: None,
            source_model: Some("openai:gpt-test".to_string()),
        }
        .is_llm_enriched()
    );

    assert!(
        !SemanticSummarySnapshot {
            semantic_features_input_hash: "hash-1".to_string(),
            summary: "Function load user.".to_string(),
            llm_summary: None,
            source_model: None,
        }
        .is_llm_enriched()
    );
}

#[test]
fn strict_summary_provider_rejects_template_only_rows() {
    let rows = semantic_rows(None, "Defines the rust source file.", Some(0.35), None);

    let err = ensure_required_llm_summary_output(&rows, &StrictNoopSummaryProvider)
        .expect_err("strict provider should reject template-only summaries");
    assert!(
        err.to_string()
            .contains("configured semantic summary provider returned no model-backed summary")
    );
}

#[test]
fn strict_summary_provider_accepts_model_backed_rows() {
    let rows = semantic_rows(
        Some("Summarises the symbol."),
        "Defines the rust source file. Summarises the symbol.",
        None,
        Some("ollama:ministral-3:3b"),
    );

    ensure_required_llm_summary_output(&rows, &StrictNoopSummaryProvider)
        .expect("strict provider should accept model-backed summaries");
}
