use std::collections::HashSet;
use std::fs;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub success: bool,
    pub mode: String,
    pub parser_version: String,
    pub extractor_version: String,
    pub active_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub head_tree_sha: Option<String>,
    pub paths_unchanged: usize,
    pub paths_added: usize,
    pub paths_changed: usize,
    pub paths_removed: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub parse_errors: usize,
}

pub async fn run_sync(cfg: &DevqlConfig, mode: sync::types::SyncMode) -> Result<()> {
    run_sync_with_summary(cfg, mode).await.map(|_| ())
}

pub async fn run_sync_with_summary(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
) -> Result<SyncSummary> {
    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
        .context("resolving DevQL backend config for `devql sync`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql sync").await?;
    init_relational_schema(cfg, &relational).await?;

    execute_sync(cfg, &relational, mode).await
}

pub(crate) async fn execute_sync(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
) -> Result<SyncSummary> {
    let (parser_version, extractor_version) = resolve_pack_versions()?;
    let _lock =
        sync::lock::SyncLock::acquire(&cfg.config_root).context("acquiring DevQL sync lock")?;

    sync::lock::write_sync_started(
        relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        sync_reason(&mode),
        &parser_version,
        &extractor_version,
    )
    .await?;

    match execute_sync_inner(cfg, relational, &mode, &parser_version, &extractor_version).await {
        Ok(summary) => {
            sync::lock::write_sync_completed(
                relational,
                &cfg.repo.repo_id,
                summary.head_commit_sha.as_deref(),
                summary.head_tree_sha.as_deref(),
                summary.active_branch.as_deref(),
                &summary.parser_version,
                &summary.extractor_version,
            )
            .await?;
            Ok(summary)
        }
        Err(err) => {
            if let Err(write_err) =
                sync::lock::write_sync_failed(relational, &cfg.repo.repo_id).await
            {
                log::warn!(
                    "failed to mark DevQL sync as failed for repo `{}`: {write_err:#}",
                    cfg.repo.repo_id
                );
            }
            Err(err)
        }
    }
}

async fn execute_sync_inner(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: &sync::types::SyncMode,
    parser_version: &str,
    extractor_version: &str,
) -> Result<SyncSummary> {
    // Phase 0: inspect the current workspace and resolve any path scope.
    let workspace = sync::workspace_state::inspect_workspace(&cfg.repo_root)
        .context("inspecting workspace for DevQL sync")?;
    let requested_paths = requested_paths(mode);

    // Phase 1: build the desired manifest for supported source files only.
    let mut desired = sync::manifest::build_desired_manifest(&workspace, &cfg.repo_root, |path| {
        resolve_language_id_for_file_path(path).map(str::to_string)
    })
    .context("building desired manifest for DevQL sync")?;
    if let Some(requested_paths) = requested_paths.as_ref() {
        desired.retain(|path, _| requested_paths.contains(path));
    }

    // Phase 2: load the stored manifest from current sync state.
    let mut stored = load_stored_manifest(relational, &cfg.repo.repo_id)
        .await
        .context("loading stored manifest for DevQL sync")?;
    if let Some(requested_paths) = requested_paths.as_ref() {
        stored.retain(|path, _| requested_paths.contains(path));
    }

    // Phase 3: classify path changes for this sync run.
    let classified = sync::manifest::classify_paths(
        &desired,
        &stored,
        parser_version,
        extractor_version,
        matches!(mode, sync::types::SyncMode::Repair),
    );

    let mut counters = sync::types::SyncCounters::default();
    for path in &classified {
        match path.action {
            sync::types::PathAction::Unchanged => counters.paths_unchanged += 1,
            sync::types::PathAction::Added => counters.paths_added += 1,
            sync::types::PathAction::Changed => counters.paths_changed += 1,
            sync::types::PathAction::Removed => counters.paths_removed += 1,
        }
    }

    // Phase 4: remove paths that no longer exist in the effective manifest.
    for path in classified
        .iter()
        .filter(|path| matches!(path.action, sync::types::PathAction::Removed))
    {
        sync::materializer::remove_path(cfg, relational, &path.path)
            .await
            .with_context(|| format!("removing stale path `{}` during DevQL sync", path.path))?;
    }

    // Phase 5: hydrate cached extraction payloads or re-read effective content.
    let mut staged_materializations = Vec::new();
    for path in classified.iter().filter(|path| {
        matches!(
            path.action,
            sync::types::PathAction::Added | sync::types::PathAction::Changed
        )
    }) {
        let desired = path
            .desired
            .clone()
            .ok_or_else(|| anyhow!("missing desired state for sync path `{}`", path.path))?;

        let extraction = match sync::content_cache::lookup_cached_content(
            relational,
            &desired.effective_content_id,
            &desired.language,
            parser_version,
            extractor_version,
        )
        .await
        .with_context(|| format!("looking up cached extraction for `{}`", desired.path))?
        {
            Some(cached) => {
                counters.cache_hits += 1;
                cached
            }
            None => {
                counters.cache_misses += 1;
                let content = read_effective_content(cfg, &desired)
                    .with_context(|| format!("reading effective content for `{}`", desired.path))?;
                let Some(extraction) = sync::extraction::extract_to_cache_format(
                    cfg,
                    &desired.path,
                    &desired.effective_content_id,
                    parser_version,
                    extractor_version,
                    &content,
                )
                .with_context(|| format!("extracting `{}` into sync cache format", desired.path))?
                else {
                    counters.parse_errors += 1;
                    continue;
                };

                sync::content_cache::store_cached_content(
                    relational,
                    &extraction,
                    determine_retention_class(&desired),
                )
                .await
                .with_context(|| format!("storing cached extraction for `{}`", desired.path))?;
                extraction
            }
        };

        staged_materializations.push((desired, extraction));
    }

    // Phase 6: materialize added and changed paths into current-state tables.
    for (desired, extraction) in staged_materializations {
        sync::materializer::materialize_path(
            cfg,
            relational,
            &desired,
            &extraction,
            parser_version,
            extractor_version,
        )
        .await
        .with_context(|| format!("materializing `{}` during DevQL sync", desired.path))?;
    }

    // Phase 7: emit the final summary with the resolved workspace identity.
    Ok(SyncSummary {
        success: true,
        mode: sync_reason(mode).to_string(),
        parser_version: parser_version.to_string(),
        extractor_version: extractor_version.to_string(),
        active_branch: workspace.active_branch,
        head_commit_sha: workspace.head_commit_sha,
        head_tree_sha: workspace.head_tree_sha,
        paths_unchanged: counters.paths_unchanged,
        paths_added: counters.paths_added,
        paths_changed: counters.paths_changed,
        paths_removed: counters.paths_removed,
        cache_hits: counters.cache_hits,
        cache_misses: counters.cache_misses,
        parse_errors: counters.parse_errors,
    })
}

fn sync_reason(mode: &sync::types::SyncMode) -> &'static str {
    match mode {
        sync::types::SyncMode::Auto => "full",
        sync::types::SyncMode::Full => "full",
        sync::types::SyncMode::Paths(_) => "paths",
        sync::types::SyncMode::Repair => "repair",
    }
}

fn requested_paths(mode: &sync::types::SyncMode) -> Option<HashSet<String>> {
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

fn resolve_pack_versions() -> Result<(String, String)> {
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

async fn load_stored_manifest(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<sync::types::StoredManifest> {
    let rows = relational
        .query_rows(&format!(
            "SELECT path, language, effective_content_id, parser_version, extractor_version \
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

    Some(sync::types::StoredFileState {
        path,
        language: row
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        effective_content_id,
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

fn read_effective_content(
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

fn determine_retention_class(desired: &sync::types::DesiredFileState) -> &'static str {
    match desired.effective_source {
        sync::types::EffectiveSource::Worktree => "worktree_only",
        sync::types::EffectiveSource::Index => "git_backed",
        sync::types::EffectiveSource::Head => "git_backed",
    }
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
}
