use crate::models::{
    CurrentCanonicalArtefactRecord, CurrentCanonicalEdgeRecord, CurrentCanonicalFileRecord,
};

use super::*;

fn file_fixture(path: &str) -> CurrentCanonicalFileRecord {
    CurrentCanonicalFileRecord {
        repo_id: "repo-1".to_string(),
        path: path.to_string(),
        analysis_mode: "code".to_string(),
        file_role: "source".to_string(),
        language: "rust".to_string(),
        resolved_language: "rust".to_string(),
        effective_content_id: format!("content:{path}"),
        parser_version: "parser".to_string(),
        extractor_version: "extractor".to_string(),
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn artefact_fixture(path: &str, name: &str) -> CurrentCanonicalArtefactRecord {
    CurrentCanonicalArtefactRecord {
        repo_id: "repo-1".to_string(),
        path: path.to_string(),
        content_id: format!("content:{path}"),
        symbol_id: format!("symbol-{name}"),
        artefact_id: format!("artefact-{name}"),
        language: "rust".to_string(),
        extraction_fingerprint: format!("fingerprint-{name}"),
        canonical_kind: Some("function".to_string()),
        language_kind: Some("function".to_string()),
        symbol_fqn: Some(format!("crate::{name}")),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 5,
        start_byte: 0,
        end_byte: 40,
        signature: None,
        modifiers: String::new(),
        docstring: None,
    }
}

#[test]
fn extracts_file_and_path_facts_for_affected_path() -> anyhow::Result<()> {
    let files = vec![file_fixture("bitloops/src/cli/main.rs")];
    let affected_paths = BTreeSet::from(["bitloops/src/cli/main.rs".to_string()]);
    let artefacts = Vec::new();
    let dependency_edges = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;

    assert!(result.facts.iter().any(|fact| {
        fact.fact_kind == "path" && fact.fact_key == "segment" && fact.fact_value == "cli"
    }));
    assert!(result.facts.iter().any(|fact| {
        fact.fact_kind == "language" && fact.fact_key == "resolved" && fact.fact_value == "rust"
    }));
    assert_eq!(result.refreshed_paths, vec!["bitloops/src/cli/main.rs"]);
    Ok(())
}

#[test]
fn extracts_path_and_language_facts_for_artefact_targets() -> anyhow::Result<()> {
    let artefacts = vec![artefact_fixture(
        "src/application/create_user.rs",
        "create_user_use_case",
    )];
    let files = Vec::new();
    let dependency_edges = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let affected_paths = BTreeSet::from(["src/application/create_user.rs".to_string()]);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;

    let artefact_target = RoleTarget::artefact(
        "artefact-create_user_use_case".to_string(),
        "symbol-create_user_use_case".to_string(),
        "src/application/create_user.rs".to_string(),
    );
    let target_facts = result
        .facts
        .iter()
        .filter(|fact| fact.target == artefact_target)
        .collect::<Vec<_>>();

    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "path"
            && fact.fact_key == "full"
            && fact.fact_value == "src/application/create_user.rs"
    }));
    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "path" && fact.fact_key == "segment" && fact.fact_value == "application"
    }));
    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "path" && fact.fact_key == "extension" && fact.fact_value == "rs"
    }));
    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "language" && fact.fact_key == "declared" && fact.fact_value == "rust"
    }));
    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "language" && fact.fact_key == "resolved" && fact.fact_value == "rust"
    }));
    assert!(target_facts.iter().any(|fact| {
        fact.fact_kind == "symbol"
            && fact.fact_key == "fqn"
            && fact.fact_value == "crate::create_user_use_case"
    }));
    Ok(())
}

#[test]
fn refreshed_paths_include_dependency_destinations_for_affected_source_path() -> anyhow::Result<()>
{
    let artefacts = vec![
        artefact_fixture("src/a.rs", "a"),
        artefact_fixture("src/b.rs", "b"),
    ];
    let dependency_edges = vec![CurrentCanonicalEdgeRecord {
        repo_id: "repo-1".to_string(),
        edge_id: "edge-1".to_string(),
        path: "src/a.rs".to_string(),
        content_id: "content-a".to_string(),
        from_symbol_id: "symbol-a".to_string(),
        from_artefact_id: "artefact-a".to_string(),
        to_symbol_id: Some("symbol-b".to_string()),
        to_artefact_id: Some("artefact-b".to_string()),
        to_symbol_ref: Some("crate::b::destination".to_string()),
        edge_kind: "calls".to_string(),
        language: "rust".to_string(),
        start_line: Some(2),
        end_line: Some(2),
        metadata: "{}".to_string(),
    }];
    let files = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;

    assert_eq!(result.refreshed_paths, vec!["src/a.rs", "src/b.rs"]);
    assert!(result.facts.iter().any(|fact| {
        fact.target.path == "src/b.rs"
            && fact.fact_kind == "dependency"
            && fact.fact_key == "incoming_kind"
            && fact.fact_value == "calls"
    }));
    Ok(())
}

#[test]
fn affected_artefact_path_refreshes_all_artefact_paths_for_removed_edges() -> anyhow::Result<()> {
    let artefacts = vec![
        artefact_fixture("src/a.rs", "a"),
        artefact_fixture("src/b.rs", "b"),
    ];
    let files = Vec::new();
    let dependency_edges = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;

    assert_eq!(result.refreshed_paths, vec!["src/a.rs", "src/b.rs"]);
    Ok(())
}

#[test]
fn affected_artefact_path_refreshes_all_artefact_paths_for_partial_edge_removals()
-> anyhow::Result<()> {
    let artefacts = vec![
        artefact_fixture("src/a.rs", "a"),
        artefact_fixture("src/b.rs", "b"),
        artefact_fixture("src/c.rs", "c"),
    ];
    let dependency_edges = vec![CurrentCanonicalEdgeRecord {
        repo_id: "repo-1".to_string(),
        edge_id: "edge-1".to_string(),
        path: "src/a.rs".to_string(),
        content_id: "content-a".to_string(),
        from_symbol_id: "symbol-a".to_string(),
        from_artefact_id: "artefact-a".to_string(),
        to_symbol_id: Some("symbol-b".to_string()),
        to_artefact_id: Some("artefact-b".to_string()),
        to_symbol_ref: Some("crate::b".to_string()),
        edge_kind: "calls".to_string(),
        language: "rust".to_string(),
        start_line: Some(2),
        end_line: Some(2),
        metadata: "{}".to_string(),
    }];
    let files = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;

    assert_eq!(
        result.refreshed_paths,
        vec!["src/a.rs", "src/b.rs", "src/c.rs"]
    );
    Ok(())
}

#[test]
fn extracts_facts_in_stable_order_independent_of_input_order() -> anyhow::Result<()> {
    let first_files = vec![file_fixture("src/a.rs"), file_fixture("src/b.rs")];
    let second_files = vec![file_fixture("src/b.rs"), file_fixture("src/a.rs")];
    let affected_paths = BTreeSet::new();
    let artefacts = Vec::new();
    let dependency_edges = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);

    let first = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &first_files,
        },
        &source,
    )?;
    let second = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &second_files,
        },
        &source,
    )?;

    assert_eq!(first.facts, second.facts);
    Ok(())
}

#[test]
fn dependency_counts_are_totalled_once_per_target() -> anyhow::Result<()> {
    let artefacts = vec![
        artefact_fixture("src/a.rs", "a"),
        artefact_fixture("src/b.rs", "b"),
        artefact_fixture("src/c.rs", "c"),
    ];
    let dependency_edges = vec![
        CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: "edge-1".to_string(),
            path: "src/a.rs".to_string(),
            content_id: "content-a".to_string(),
            from_symbol_id: "symbol-a".to_string(),
            from_artefact_id: "artefact-a".to_string(),
            to_symbol_id: Some("symbol-b".to_string()),
            to_artefact_id: Some("artefact-b".to_string()),
            to_symbol_ref: Some("crate::b".to_string()),
            edge_kind: "calls".to_string(),
            language: "rust".to_string(),
            start_line: Some(2),
            end_line: Some(2),
            metadata: "{}".to_string(),
        },
        CurrentCanonicalEdgeRecord {
            repo_id: "repo-1".to_string(),
            edge_id: "edge-2".to_string(),
            path: "src/a.rs".to_string(),
            content_id: "content-a".to_string(),
            from_symbol_id: "symbol-a".to_string(),
            from_artefact_id: "artefact-a".to_string(),
            to_symbol_id: Some("symbol-c".to_string()),
            to_artefact_id: Some("artefact-c".to_string()),
            to_symbol_ref: Some("crate::c".to_string()),
            edge_kind: "imports".to_string(),
            language: "rust".to_string(),
            start_line: Some(3),
            end_line: Some(3),
            metadata: "{}".to_string(),
        },
    ];
    let files = Vec::new();
    let source = SliceArchitectureRoleCurrentStateSource::new(&artefacts, &dependency_edges);
    let affected_paths = BTreeSet::from(["src/a.rs".to_string()]);

    let result = extract_architecture_role_facts(
        ArchitectureRoleFactExtractionInput {
            repo_id: "repo-1",
            generation_seq: 10,
            affected_paths: &affected_paths,
            files: &files,
        },
        &source,
    )?;
    let outgoing_counts = result
        .facts
        .iter()
        .filter(|fact| {
            fact.target.path == "src/a.rs"
                && fact.fact_kind == "dependency"
                && fact.fact_key == "outgoing_count"
        })
        .collect::<Vec<_>>();

    assert_eq!(outgoing_counts.len(), 1);
    assert_eq!(outgoing_counts[0].fact_value, "2");
    assert!(result.facts.iter().any(|fact| {
        fact.target.path == "src/a.rs"
            && fact.fact_kind == "dependency"
            && fact.fact_key == "outgoing_kind"
            && fact.fact_value == "calls"
    }));
    assert!(result.facts.iter().any(|fact| {
        fact.target.path == "src/a.rs"
            && fact.fact_kind == "dependency"
            && fact.fact_key == "outgoing_kind"
            && fact.fact_value == "imports"
    }));
    Ok(())
}
