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

pub(super) async fn load_stored_manifest(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<sync::types::StoredManifest> {
    let rows = relational
        .query_rows(&format!(
            "SELECT path, language, effective_content_id, effective_source, parser_version, extractor_version \
             FROM current_file_state \
             WHERE repo_id = '{}' \
             ORDER BY path",
            esc_pg(repo_id),
        ))
        .await?;

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

    Some(sync::types::StoredFileState {
        path,
        language: row
            .get("language")
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

pub(super) fn read_effective_content(
    cfg: &DevqlConfig,
    desired: &sync::types::DesiredFileState,
) -> Result<String> {
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
            String::from_utf8(raw)
                .with_context(|| format!("decoding `{}` from worktree as UTF-8", desired.path))
        }
    }
}

fn read_blob_content(
    repo_root: &std::path::Path,
    blob_sha: &str,
    path: &str,
    source: &str,
) -> Result<String> {
    super::git_blob_content(repo_root, blob_sha)
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
            language: "rust".to_string(),
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
