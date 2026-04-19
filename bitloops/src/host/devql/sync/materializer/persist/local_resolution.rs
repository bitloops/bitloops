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
}

fn compatible_resolution_languages(language: &str) -> Vec<&'static str> {
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
    let in_list = compatible_languages
        .iter()
        .map(|language| format!("'{}'", crate::host::devql::esc_pg(language)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id, language_kind \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND language IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
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
    let in_list = compatible_languages
        .iter()
        .map(|language| format!("'{}'", crate::host::devql::esc_pg(language)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT symbol_fqn, symbol_id, artefact_id, language_kind \
         FROM artefacts_current \
         WHERE repo_id = '{}' AND path != '{}' AND language IN ({in_list})",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(current_path),
    );
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
