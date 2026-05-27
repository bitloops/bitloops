use anyhow::{Context, Result};
use rusqlite::Connection;

use super::super::super::types::DesiredFileState;
use super::super::types::PreparedMaterialisationRows;

pub(super) async fn resolve_prepared_local_edges(
    cfg: &crate::host::devql::DevqlConfig,
    relational: &crate::host::devql::RelationalStorage,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let source_facts = source_facts_from_materialized_rows(desired.path.as_str(), prepared);
    let current_targets = load_current_targets_for_resolution(
        relational,
        &cfg.repo.repo_id,
        &desired.path,
        &desired.language,
    )
    .await?;
    apply_local_edge_resolutions(cfg, desired, prepared, &source_facts, &current_targets);
    Ok(())
}

pub(crate) fn resolve_prepared_local_edges_with_connection(
    connection: &Connection,
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
) -> Result<()> {
    let source_facts = source_facts_from_materialized_rows(desired.path.as_str(), prepared);
    let current_targets = load_current_targets_for_resolution_with_connection(
        connection,
        &cfg.repo.repo_id,
        &desired.path,
        &desired.language,
    )?;
    apply_local_edge_resolutions(cfg, desired, prepared, &source_facts, &current_targets);
    Ok(())
}

fn source_facts_from_materialized_rows(
    source_path: &str,
    prepared: &PreparedMaterialisationRows,
) -> crate::host::language_adapter::LocalSourceFacts {
    let import_refs = prepared
        .materialized_edges
        .iter()
        .filter(|edge| edge.edge_kind == "imports")
        .filter_map(|edge| edge.to_symbol_ref.clone())
        .collect::<Vec<_>>();
    let package_refs = prepared
        .materialized_artefacts
        .iter()
        .filter(|artefact| {
            artefact.symbol_fqn.starts_with(&format!("{source_path}::"))
                && artefact.language_kind == "package_declaration"
        })
        .filter_map(|artefact| {
            artefact
                .symbol_fqn
                .split_once("::")
                .map(|(_, package)| package.to_string())
        })
        .collect::<Vec<_>>();
    let namespace_refs = prepared
        .materialized_artefacts
        .iter()
        .filter(|artefact| {
            artefact
                .symbol_fqn
                .starts_with(&format!("{source_path}::ns::"))
                && matches!(
                    artefact.language_kind.as_str(),
                    "namespace_declaration" | "file_scoped_namespace_declaration"
                )
        })
        .filter_map(|artefact| {
            artefact
                .symbol_fqn
                .split_once("::ns::")
                .map(|(_, namespace)| namespace.to_string())
        })
        .collect::<Vec<_>>();

    crate::host::language_adapter::LocalSourceFacts {
        import_refs,
        package_refs,
        namespace_refs,
    }
}

fn in_flight_local_targets(
    prepared: &PreparedMaterialisationRows,
) -> Vec<crate::host::language_adapter::LocalTargetInfo> {
    prepared
        .materialized_artefacts
        .iter()
        .map(|artefact| crate::host::language_adapter::LocalTargetInfo {
            symbol_fqn: artefact.symbol_fqn.clone(),
            symbol_id: artefact.symbol_id.clone(),
            artefact_id: artefact.artefact_id.clone(),
            language_kind: artefact.language_kind.clone(),
        })
        .collect()
}

fn apply_local_edge_resolutions(
    cfg: &crate::host::devql::DevqlConfig,
    desired: &DesiredFileState,
    prepared: &mut PreparedMaterialisationRows,
    source_facts: &crate::host::language_adapter::LocalSourceFacts,
    current_targets: &[crate::host::language_adapter::LocalTargetInfo],
) {
    let mut targets = in_flight_local_targets(prepared);
    targets.extend_from_slice(current_targets);

    for edge in &mut prepared.materialized_edges {
        if edge.to_symbol_id.is_some() {
            continue;
        }
        let Some(symbol_ref) = edge.to_symbol_ref.as_deref() else {
            continue;
        };
        let Some(resolved) = crate::host::language_adapter::resolve_local_symbol_ref(
            &edge.language,
            desired.path.as_str(),
            &edge.edge_kind,
            symbol_ref,
            source_facts,
            &targets,
        ) else {
            continue;
        };

        edge.edge_kind = resolved.edge_kind;
        edge.to_symbol_id = Some(resolved.symbol_id);
        edge.to_artefact_id = Some(resolved.artefact_id);
        edge.to_symbol_ref = Some(resolved.symbol_fqn);
        edge.edge_id = crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            cfg.repo.repo_id,
            desired.path,
            edge.from_symbol_id,
            edge.edge_kind,
            edge.to_symbol_id.clone().unwrap_or_default(),
            edge.to_symbol_ref.clone().unwrap_or_default(),
            edge.start_line.unwrap_or(-1),
            edge.end_line.unwrap_or(-1),
            edge.metadata,
        ));
    }

    // Resolution can collapse two distinct pre-resolution refs (e.g. `cli::TestOpts`
    // and `crate::cli::TestOpts`) onto the same canonical target, producing identical
    // post-resolution edge_ids. The pre-resolution dedup in `prepare_materialization_rows`
    // does not catch these, so dedup again here to uphold the (repo_id, edge_id) primary
    // key on `artefact_edges_current`. By construction every "duplicate" carries
    // identical data, so retaining the first occurrence is safe.
    let mut seen = std::collections::HashSet::<String>::new();
    prepared
        .materialized_edges
        .retain(|edge| seen.insert(edge.edge_id.clone()));
}

pub(super) fn compatible_resolution_languages(language: &str) -> Vec<&'static str> {
    match language.trim().to_ascii_lowercase().as_str() {
        "typescript" | "javascript" => vec!["typescript", "javascript"],
        "rust" => vec!["rust"],
        "python" => vec!["python"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        "csharp" | "c#" => vec!["csharp"],
        _ => vec![],
    }
}

fn current_targets_for_compatible_languages_sql(
    repo_id: &str,
    excluded_path: Option<&str>,
    compatible_languages: &[&str],
) -> String {
    let repo_filter = format!("repo_id = '{}'", crate::host::devql::esc_pg(repo_id));
    let path_filter = excluded_path
        .map(|path| format!(" AND path != '{}'", crate::host::devql::esc_pg(path)))
        .unwrap_or_default();
    let in_list = compatible_languages
        .iter()
        .map(|language| format!("'{}'", crate::host::devql::esc_pg(language)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT symbol_fqn, symbol_id, artefact_id, language_kind \
         FROM artefacts_current \
         WHERE {repo_filter}{path_filter} AND language IN ({in_list})",
    )
}

async fn load_current_targets_for_compatible_languages(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    excluded_path: Option<&str>,
    compatible_languages: &[&str],
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let sql =
        current_targets_for_compatible_languages_sql(repo_id, excluded_path, compatible_languages);
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let obj = row.as_object()?;
            Some(crate::host::language_adapter::LocalTargetInfo {
                symbol_fqn: obj.get("symbol_fqn")?.as_str()?.to_string(),
                symbol_id: obj.get("symbol_id")?.as_str()?.to_string(),
                artefact_id: obj.get("artefact_id")?.as_str()?.to_string(),
                language_kind: obj.get("language_kind")?.as_str()?.to_string(),
            })
        })
        .collect())
}

fn load_current_targets_for_compatible_languages_with_connection(
    connection: &Connection,
    repo_id: &str,
    excluded_path: Option<&str>,
    compatible_languages: &[&str],
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let sql =
        current_targets_for_compatible_languages_sql(repo_id, excluded_path, compatible_languages);
    let mut stmt = connection
        .prepare(&sql)
        .context("preparing current local target lookup query")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(crate::host::language_adapter::LocalTargetInfo {
                symbol_fqn: row.get::<_, String>(0)?,
                symbol_id: row.get::<_, String>(1)?,
                artefact_id: row.get::<_, String>(2)?,
                language_kind: row.get::<_, String>(3)?,
            })
        })
        .context("querying current local target lookup rows")?
        .collect::<Result<Vec<_>, _>>()
        .context("collecting current local target lookup rows")?;
    Ok(rows)
}

async fn load_current_targets_for_resolution(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    current_path: &str,
    language: &str,
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let compatible_languages = compatible_resolution_languages(language);
    if compatible_languages.is_empty() {
        return Ok(Vec::new());
    }
    load_current_targets_for_compatible_languages(
        relational,
        repo_id,
        Some(current_path),
        &compatible_languages,
    )
    .await
}

pub(super) fn load_current_targets_for_resolution_with_connection(
    connection: &Connection,
    repo_id: &str,
    current_path: &str,
    language: &str,
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    let compatible_languages = compatible_resolution_languages(language);
    if compatible_languages.is_empty() {
        return Ok(Vec::new());
    }
    load_current_targets_for_compatible_languages_with_connection(
        connection,
        repo_id,
        Some(current_path),
        &compatible_languages,
    )
}

pub(super) fn load_current_targets_for_languages_with_connection(
    connection: &Connection,
    repo_id: &str,
    compatible_languages: &[&str],
) -> Result<Vec<crate::host::language_adapter::LocalTargetInfo>> {
    if compatible_languages.is_empty() {
        return Ok(Vec::new());
    }
    load_current_targets_for_compatible_languages_with_connection(
        connection,
        repo_id,
        None,
        compatible_languages,
    )
}
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::apply_local_edge_resolutions;
    use crate::host::devql::sync::materializer::{MaterializedEdge, PreparedMaterialisationRows};
    use crate::host::devql::sync::types::{DesiredFileState, EffectiveSource};
    use crate::host::devql::{AnalysisMode, FileRole, RepoIdentity, TextIndexMode};
    use crate::host::language_adapter::{LocalSourceFacts, LocalTargetInfo};

    fn test_cfg() -> crate::host::devql::DevqlConfig {
        crate::host::devql::DevqlConfig {
            daemon_config_root: PathBuf::from("/tmp/local-resolution-test"),
            repo_root: PathBuf::from("/tmp/local-resolution-test"),
            repo: RepoIdentity {
                provider: "github".to_string(),
                organization: "bitloops".to_string(),
                name: "local-resolution-test".to_string(),
                identity: "github/bitloops/local-resolution-test".to_string(),
                repo_id: "repo-local-resolution-test".to_string(),
            },
            pg_dsn: None,
            clickhouse_url: "http://localhost:8123".to_string(),
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: "default".to_string(),
        }
    }

    fn desired_rust_file(path: &str) -> DesiredFileState {
        DesiredFileState {
            path: path.to_string(),
            analysis_mode: AnalysisMode::Code,
            file_role: FileRole::SourceCode,
            text_index_mode: TextIndexMode::None,
            language: "rust".to_string(),
            resolved_language: "rust".to_string(),
            dialect: None,
            primary_context_id: None,
            secondary_context_ids: Vec::new(),
            frameworks: Vec::new(),
            runtime_profile: None,
            classification_reason: "test".to_string(),
            context_fingerprint: None,
            extraction_fingerprint: "fingerprint-v1".to_string(),
            head_content_id: Some("content".to_string()),
            index_content_id: Some("content".to_string()),
            worktree_content_id: Some("content".to_string()),
            effective_content_id: "content".to_string(),
            effective_source: EffectiveSource::Head,
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        }
    }

    fn unresolved_imports_edge(symbol_ref: &str) -> MaterializedEdge {
        MaterializedEdge {
            // The pre-resolution edge_ids differ because `to_symbol_ref` differs;
            // we mimic what `derive_materialized_edges` would have produced.
            edge_id: format!("pre::{symbol_ref}"),
            from_symbol_id: "file-symbol".to_string(),
            from_artefact_id: "file-artefact".to_string(),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: Some(symbol_ref.to_string()),
            edge_kind: "imports".to_string(),
            language: "rust".to_string(),
            // Match the real-world trigger: re-export edges with no line info.
            start_line: None,
            end_line: None,
            metadata: Value::Object(Default::default()),
        }
    }

    /// Regression test for the `library/test/src/lib.rs` sync failure on
    /// `rust-lang/rust`: two unresolved `imports` edges with different
    /// pre-resolution `to_symbol_ref` strings (`crate::cli::TestOpts` and
    /// `self::cli::TestOpts`) resolve to the same canonical local target.
    /// Before the post-resolution dedup, both edges ended up with the same
    /// `edge_id`, which crashed the SQLite writer on the
    /// `PRIMARY KEY (repo_id, edge_id)` constraint of `artefact_edges_current`.
    #[test]
    fn apply_local_edge_resolutions_dedups_post_resolution_collisions() {
        let cfg = test_cfg();
        let desired = desired_rust_file("src/lib.rs");
        let mut prepared = PreparedMaterialisationRows {
            materialized_artefacts: Vec::new(),
            materialized_edges: vec![
                unresolved_imports_edge("crate::cli::TestOpts"),
                unresolved_imports_edge("self::cli::TestOpts"),
            ],
        };

        // The two refs share a canonical local target. The resolver's Rust
        // import path produces `src/cli.rs::TestOpts` for both refs from a
        // file at `src/lib.rs`.
        let targets = vec![LocalTargetInfo {
            symbol_fqn: "src/cli.rs::TestOpts".to_string(),
            symbol_id: "cli-symbol".to_string(),
            artefact_id: "cli-artefact".to_string(),
            language_kind: "struct_item".to_string(),
        }];
        let source_facts = LocalSourceFacts::default();

        // Sanity: the two pre-resolution edges are distinct (different edge_ids).
        let pre_ids: std::collections::HashSet<_> = prepared
            .materialized_edges
            .iter()
            .map(|edge| edge.edge_id.clone())
            .collect();
        assert_eq!(
            pre_ids.len(),
            2,
            "test fixture should start with two distinct pre-resolution edges"
        );

        apply_local_edge_resolutions(&cfg, &desired, &mut prepared, &source_facts, &targets);

        assert_eq!(
            prepared.materialized_edges.len(),
            1,
            "post-resolution dedup should collapse both refs to a single edge; \
             without the fix this would be 2 and crash the SQLite writer on \
             `UNIQUE constraint failed: artefact_edges_current.repo_id, artefact_edges_current.edge_id`"
        );
        let edge = &prepared.materialized_edges[0];
        assert_eq!(edge.to_symbol_id.as_deref(), Some("cli-symbol"));
        assert_eq!(edge.to_artefact_id.as_deref(), Some("cli-artefact"));
        assert_eq!(edge.to_symbol_ref.as_deref(), Some("src/cli.rs::TestOpts"));
    }

    /// Edges that the resolver does not touch must keep their original
    /// edge_ids and not be deduped against each other (they were already
    /// unique pre-resolution).
    #[test]
    fn apply_local_edge_resolutions_preserves_distinct_unresolved_edges() {
        let cfg = test_cfg();
        let desired = desired_rust_file("src/lib.rs");
        // Two edges with refs the Rust resolver cannot reach — bare names,
        // no `crate::` / `self::` / `super::` prefix.
        let mut prepared = PreparedMaterialisationRows {
            materialized_artefacts: Vec::new(),
            materialized_edges: vec![
                unresolved_imports_edge("unrelated_a::Thing"),
                unresolved_imports_edge("unrelated_b::Thing"),
            ],
        };
        let source_facts = LocalSourceFacts::default();
        let targets: Vec<LocalTargetInfo> = Vec::new();

        apply_local_edge_resolutions(&cfg, &desired, &mut prepared, &source_facts, &targets);

        assert_eq!(
            prepared.materialized_edges.len(),
            2,
            "edges that fail to resolve must not be collapsed"
        );
    }
}
