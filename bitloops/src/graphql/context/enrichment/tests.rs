use super::*;
use crate::artefact_query_planner::{
    ArtefactQuerySpec, ArtefactScope, ArtefactStructuralFilter, ArtefactTemporalScope,
};
use crate::capability_packs::semantic_clones::scoring::SymbolCloneEdgeRow;
use crate::graphql::types::ClonesFilterInput;
use serde_json::json;

fn clone_summary_spec() -> ArtefactQuerySpec {
    ArtefactQuerySpec {
        repo_id: "repo-1".to_string(),
        branch: Some("main".to_string()),
        historical_path_blob_sha: None,
        scope: ArtefactScope {
            project_path: Some("packages/api".to_string()),
            path: None,
            files_path: None,
        },
        temporal_scope: ArtefactTemporalScope::Current,
        structural_filter: ArtefactStructuralFilter::default(),
        activity_filter: None,
        pagination: None,
    }
}

#[test]
fn clone_from_row_preserves_metadata_and_missing_spans() {
    let clone = clone_from_row(json!({
        "source_artefact_id": "artefact::source",
        "target_artefact_id": "artefact::target",
        "relation_kind": "similar_implementation",
        "score": 0.9,
        "semantic_score": 0.8,
        "lexical_score": 0.7,
        "structural_score": 0.6,
        "explanation_json": "{\"reason\":\"shared structure\"}"
    }))
    .expect("clone row parses");

    assert_eq!(clone.source_start_line, None);
    assert_eq!(clone.source_end_line, None);
    assert_eq!(clone.target_start_line, None);
    assert_eq!(clone.target_end_line, None);
    let metadata = clone.metadata.expect("metadata should be present");
    assert_eq!(metadata.0["semanticScore"], json!(0.8));
    assert_eq!(metadata.0["lexicalScore"], json!(0.7));
    assert_eq!(metadata.0["structuralScore"], json!(0.6));
    assert_eq!(metadata.0["explanation"]["reason"], "shared structure");
}

#[test]
fn clone_edge_filter_applies_relation_and_min_score() {
    let edge = SymbolCloneEdgeRow {
        repo_id: "repo-1".to_string(),
        source_symbol_id: "sym-source".to_string(),
        source_artefact_id: "a-source".to_string(),
        target_symbol_id: "sym-target".to_string(),
        target_artefact_id: "a-target".to_string(),
        relation_kind: "similar_implementation".to_string(),
        score: 0.77,
        semantic_score: 0.8,
        lexical_score: 0.6,
        structural_score: 0.4,
        clone_input_hash: "hash".to_string(),
        explanation_json: json!({}),
    };

    let matching = ClonesFilterInput {
        relation_kind: Some("similar_implementation".to_string()),
        min_score: Some(0.75),
        neighbors: None,
    };
    assert!(clone_edge_matches_filter(&edge, Some(&matching)));

    let wrong_relation = ClonesFilterInput {
        relation_kind: Some("exact_duplicate".to_string()),
        min_score: None,
        neighbors: None,
    };
    assert!(!clone_edge_matches_filter(&edge, Some(&wrong_relation)));

    let high_min_score = ClonesFilterInput {
        relation_kind: None,
        min_score: Some(0.9),
        neighbors: None,
    };
    assert!(!clone_edge_matches_filter(&edge, Some(&high_min_score)));
}

#[test]
fn clone_from_edge_preserves_metadata_shape() {
    let edge = SymbolCloneEdgeRow {
        repo_id: "repo-1".to_string(),
        source_symbol_id: "sym-source".to_string(),
        source_artefact_id: "a-source".to_string(),
        target_symbol_id: "sym-target".to_string(),
        target_artefact_id: "a-target".to_string(),
        relation_kind: "similar_implementation".to_string(),
        score: 0.77,
        semantic_score: 0.8,
        lexical_score: 0.6,
        structural_score: 0.4,
        clone_input_hash: "hash".to_string(),
        explanation_json: json!({"labels":["preferred_local_pattern"]}),
    };

    let clone = clone_from_edge(edge).expect("clone row");
    assert_eq!(clone.relation_kind, "similar_implementation");
    assert!((clone.score - 0.77_f64).abs() < 1e-6);
    let metadata = clone
        .metadata
        .expect("metadata")
        .0
        .as_object()
        .cloned()
        .expect("metadata object");
    assert!(metadata.contains_key("semanticScore"));
    assert!(metadata.contains_key("lexicalScore"));
    assert!(metadata.contains_key("structuralScore"));
    assert!(metadata.contains_key("explanation"));
}

#[test]
fn clone_summary_sql_aggregates_filtered_sources_by_relation_kind() {
    let sql = build_clone_summary_sql(
        &clone_summary_spec(),
        Some(&ClonesFilterInput {
            relation_kind: Some("similar_implementation".to_string()),
            min_score: Some(0.75),
            neighbors: None,
        }),
    );

    assert!(sql.contains("WITH filtered AS"));
    assert!(sql.contains("FROM filtered fa"));
    assert!(sql.contains("JOIN symbol_clone_edges_current ce"));
    assert!(sql.contains("ce.source_artefact_id = fa.artefact_id"));
    assert!(sql.contains("ce.relation_kind = 'similar_implementation'"));
    assert!(sql.contains("ce.score >= 0.75"));
    assert!(sql.contains("GROUP BY ce.relation_kind"));
}
