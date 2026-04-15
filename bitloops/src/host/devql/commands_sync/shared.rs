use std::collections::HashSet;
use std::fs;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::*;

pub(super) fn sync_reason(mode: &sync::types::SyncMode) -> &'static str {
    match mode {
        sync::types::SyncMode::Auto => "full",
        sync::types::SyncMode::Full => "full",
        sync::types::SyncMode::Paths(_) => "paths",
        sync::types::SyncMode::Repair => "repair",
        sync::types::SyncMode::Validate => "validate",
    }
}

pub(super) fn requested_paths(mode: &sync::types::SyncMode) -> Option<HashSet<String>> {
    match mode {
        sync::types::SyncMode::Paths(paths) => Some(
            paths
                .iter()
                .map(String::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        _ => None,
    }
}

pub(super) fn resolve_pack_versions() -> Result<(String, String)> {
    let host = core_extension_host()?;
    let mut packs = host
        .language_packs()
        .registered_pack_ids()
        .into_iter()
        .filter_map(|pack_id| host.language_packs().resolve_pack(pack_id))
        .map(|descriptor| format!("{}@{}", descriptor.id, descriptor.version))
        .collect::<Vec<_>>();
    packs.sort();
    let joined = packs.join("+");
    Ok((
        format!("devql-sync-parser@{joined}"),
        format!("devql-sync-extractor@{joined}"),
    ))
}

pub(super) async fn load_stored_manifest_for_paths(
    relational: &RelationalStorage,
    repo_id: &str,
    requested_paths: Option<&HashSet<String>>,
) -> Result<sync::types::StoredManifest> {
    let sql = if let Some(requested_paths) = requested_paths {
        if requested_paths.is_empty() {
            "SELECT path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, effective_content_id, effective_source, parser_version, extractor_version \
             FROM current_file_state WHERE 1 = 0"
                .to_string()
        } else {
            let mut sorted_paths = requested_paths.iter().cloned().collect::<Vec<_>>();
            sorted_paths.sort();
            let path_list = sorted_paths
                .into_iter()
                .map(|path| format!("'{}'", esc_pg(&path)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SELECT path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, effective_content_id, effective_source, parser_version, extractor_version \
                 FROM current_file_state \
                 WHERE repo_id = '{}' AND path IN ({}) \
                 ORDER BY path",
                esc_pg(repo_id),
                path_list,
            )
        }
    } else {
        format!(
            "SELECT path, analysis_mode, file_role, text_index_mode, language, resolved_language, dialect, primary_context_id, secondary_context_ids_json, frameworks_json, runtime_profile, classification_reason, context_fingerprint, extraction_fingerprint, effective_content_id, effective_source, parser_version, extractor_version \
             FROM current_file_state \
             WHERE repo_id = '{}' \
             ORDER BY path",
            esc_pg(repo_id),
        )
    };
    let rows = relational.query_rows(&sql).await?;

    let manifest = rows
        .into_iter()
        .filter_map(|row| row.as_object().cloned())
        .filter_map(|row| stored_manifest_row(&row))
        .map(|state| (state.path.clone(), state))
        .collect::<sync::types::StoredManifest>();
    Ok(manifest)
}

fn stored_manifest_row(
    row: &serde_json::Map<String, Value>,
) -> Option<sync::types::StoredFileState> {
    let path = row.get("path").and_then(Value::as_str)?.to_string();
    let effective_content_id = row
        .get("effective_content_id")
        .and_then(Value::as_str)?
        .to_string();
    let effective_source = match row.get("effective_source").and_then(Value::as_str)? {
        "head" => sync::types::EffectiveSource::Head,
        "index" => sync::types::EffectiveSource::Index,
        "worktree" => sync::types::EffectiveSource::Worktree,
        _ => return None,
    };
    let analysis_mode = match row.get("analysis_mode").and_then(Value::as_str)? {
        "code" => crate::host::devql::AnalysisMode::Code,
        "text" => crate::host::devql::AnalysisMode::Text,
        "track_only" => crate::host::devql::AnalysisMode::TrackOnly,
        "excluded" => crate::host::devql::AnalysisMode::Excluded,
        _ => return None,
    };
    let file_role = match row
        .get("file_role")
        .and_then(Value::as_str)
        .unwrap_or("source_code")
    {
        "source_code" => crate::host::devql::FileRole::SourceCode,
        "project_manifest" => crate::host::devql::FileRole::ProjectManifest,
        "context_seed" => crate::host::devql::FileRole::ContextSeed,
        "configuration" => crate::host::devql::FileRole::Configuration,
        "documentation" => crate::host::devql::FileRole::Documentation,
        "lockfile" => crate::host::devql::FileRole::Lockfile,
        "generated" => crate::host::devql::FileRole::Generated,
        "dependency_tree" => crate::host::devql::FileRole::DependencyTree,
        "other" => crate::host::devql::FileRole::Other,
        _ => return None,
    };
    let text_index_mode = match row
        .get("text_index_mode")
        .and_then(Value::as_str)
        .unwrap_or("none")
    {
        "embed" => crate::host::devql::TextIndexMode::Embed,
        "store_only" => crate::host::devql::TextIndexMode::StoreOnly,
        "none" => crate::host::devql::TextIndexMode::None,
        _ => return None,
    };

    Some(sync::types::StoredFileState {
        path,
        analysis_mode,
        file_role,
        text_index_mode,
        language: row
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        resolved_language: row
            .get("resolved_language")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        dialect: row
            .get("dialect")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary_context_id: row
            .get("primary_context_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        secondary_context_ids: parse_json_string_array(
            row.get("secondary_context_ids_json")
                .and_then(Value::as_str),
        ),
        frameworks: parse_json_string_array(row.get("frameworks_json").and_then(Value::as_str)),
        runtime_profile: row
            .get("runtime_profile")
            .and_then(Value::as_str)
            .map(str::to_string),
        classification_reason: row
            .get("classification_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        context_fingerprint: row
            .get("context_fingerprint")
            .and_then(Value::as_str)
            .map(str::to_string),
        extraction_fingerprint: row
            .get("extraction_fingerprint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        effective_content_id,
        effective_source,
        parser_version: row
            .get("parser_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        extractor_version: row
            .get("extractor_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn parse_json_string_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

pub(super) fn read_effective_content(
    cfg: &DevqlConfig,
    desired: &sync::types::DesiredFileState,
) -> Result<DecodedFileContent> {
    match desired.effective_source {
        sync::types::EffectiveSource::Head => read_blob_content(
            &cfg.repo_root,
            desired
                .head_content_id
                .as_deref()
                .ok_or_else(|| anyhow!("missing HEAD content id for `{}`", desired.path))?,
            &desired.path,
            "HEAD",
        ),
        sync::types::EffectiveSource::Index => read_blob_content(
            &cfg.repo_root,
            desired
                .index_content_id
                .as_deref()
                .ok_or_else(|| anyhow!("missing index content id for `{}`", desired.path))?,
            &desired.path,
            "index",
        ),
        sync::types::EffectiveSource::Worktree => {
            let raw = fs::read(cfg.repo_root.join(&desired.path))
                .with_context(|| format!("reading `{}` from worktree", desired.path))?;
            Ok(DecodedFileContent::from_raw_bytes(raw))
        }
    }
}

fn read_blob_content(
    repo_root: &std::path::Path,
    blob_sha: &str,
    path: &str,
    source: &str,
) -> Result<DecodedFileContent> {
    super::git_blob_decoded_content(repo_root, blob_sha)
        .ok_or_else(|| anyhow!("missing {source} blob `{blob_sha}` for sync path `{path}`"))
}

pub(super) fn determine_retention_class(desired: &sync::types::DesiredFileState) -> &'static str {
    match desired.effective_source {
        sync::types::EffectiveSource::Worktree => "worktree_only",
        sync::types::EffectiveSource::Index => "git_backed",
        sync::types::EffectiveSource::Head => "git_backed",
    }
}

pub(super) fn is_missing_sync_schema_error(err: &anyhow::Error) -> bool {
    // Temporary safeguard while daemon-start schema bootstrap rolls out across workflows.
    // Keep this fallback so sync can still emit a direct remediation hint on legacy setups.
    let message = format!("{err:#}").to_ascii_lowercase();
    let missing_table_error = message.contains("no such table")
        || (message.contains("relation") && message.contains("does not exist"))
        || (message.contains("table") && message.contains("does not exist"));
    if !missing_table_error {
        return false;
    }

    let sync_tables = [
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
    ];
    sync_tables.iter().any(|table| message.contains(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_reason_maps_auto_to_full() {
        assert_eq!(sync_reason(&sync::types::SyncMode::Auto), "full");
        assert_eq!(sync_reason(&sync::types::SyncMode::Full), "full");
        assert_eq!(
            sync_reason(&sync::types::SyncMode::Paths(vec![
                "src/lib.rs".to_string()
            ])),
            "paths"
        );
        assert_eq!(sync_reason(&sync::types::SyncMode::Repair), "repair");
        assert_eq!(sync_reason(&sync::types::SyncMode::Validate), "validate");
    }

    #[test]
    fn determine_retention_class_matches_spec() {
        let base = sync::types::DesiredFileState {
            path: "src/lib.rs".to_string(),
            analysis_mode: crate::host::devql::AnalysisMode::Code,
            file_role: crate::host::devql::FileRole::SourceCode,
            text_index_mode: crate::host::devql::TextIndexMode::None,
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
            head_content_id: Some("head".to_string()),
            index_content_id: Some("index".to_string()),
            worktree_content_id: Some("worktree".to_string()),
            effective_content_id: "effective".to_string(),
            effective_source: sync::types::EffectiveSource::Head,
            exists_in_head: true,
            exists_in_index: true,
            exists_in_worktree: true,
        };

        let mut head = base.clone();
        head.effective_source = sync::types::EffectiveSource::Head;
        assert_eq!(determine_retention_class(&head), "git_backed");

        let mut index = base.clone();
        index.effective_source = sync::types::EffectiveSource::Index;
        assert_eq!(determine_retention_class(&index), "git_backed");

        let mut worktree = base;
        worktree.effective_source = sync::types::EffectiveSource::Worktree;
        assert_eq!(determine_retention_class(&worktree), "worktree_only");
    }

    #[test]
    fn detects_missing_sync_schema_error_shapes() {
        assert!(is_missing_sync_schema_error(&anyhow!(
            "db error: no such table: repo_sync_state"
        )));
        assert!(is_missing_sync_schema_error(&anyhow!(
            "ERROR: relation \"current_file_state\" does not exist"
        )));
        assert!(is_missing_sync_schema_error(&anyhow!(
            "Catalog Error: Table with name artefacts_current does not exist!"
        )));
        assert!(!is_missing_sync_schema_error(&anyhow!(
            "acquiring DevQL sync lock failed"
        )));
        assert!(!is_missing_sync_schema_error(&anyhow!(
            "no such table: unrelated_table"
        )));
    }
}
