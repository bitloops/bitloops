use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub validation: Option<SyncValidationSummary>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncValidationSummary {
    pub valid: bool,
    pub expected_artefacts: usize,
    pub actual_artefacts: usize,
    pub expected_edges: usize,
    pub actual_edges: usize,
    pub missing_artefacts: usize,
    pub stale_artefacts: usize,
    pub mismatched_artefacts: usize,
    pub missing_edges: usize,
    pub stale_edges: usize,
    pub mismatched_edges: usize,
    pub files_with_drift: Vec<SyncValidationFileDrift>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncValidationFileDrift {
    pub path: String,
    pub missing_artefacts: usize,
    pub stale_artefacts: usize,
    pub mismatched_artefacts: usize,
    pub missing_edges: usize,
    pub stale_edges: usize,
    pub mismatched_edges: usize,
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
    if matches!(mode, sync::types::SyncMode::Validate) {
        return match execute_sync_validation(cfg, &relational).await {
            Ok(summary) => Ok(summary),
            Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
                "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql sync --validate`.",
            ),
            Err(err) => Err(err),
        };
    }

    match execute_sync(cfg, &relational, mode).await {
        Ok(summary) => Ok(summary),
        Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
            "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql sync`.",
        ),
        Err(err) => Err(err),
    }
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
                if determine_retention_class(&desired) == "git_backed" {
                    sync::content_cache::promote_to_git_backed(
                        relational,
                        &desired.effective_content_id,
                        &desired.language,
                        parser_version,
                        extractor_version,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "promoting cached extraction retention for `{}`",
                            desired.path
                        )
                    })?;
                }
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

    if matches!(
        mode,
        sync::types::SyncMode::Auto | sync::types::SyncMode::Full
    ) {
        if let Err(err) =
            sync::gc::run_gc(relational, &cfg.repo.repo_id, sync::gc::DEFAULT_GC_TTL_DAYS).await
        {
            log::warn!(
                "failed to run DevQL cache GC for repo `{}`: {err:#}",
                cfg.repo.repo_id
            );
        }
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
        validation: None,
    })
}

fn sync_reason(mode: &sync::types::SyncMode) -> &'static str {
    match mode {
        sync::types::SyncMode::Auto => "full",
        sync::types::SyncMode::Full => "full",
        sync::types::SyncMode::Paths(_) => "paths",
        sync::types::SyncMode::Repair => "repair",
        sync::types::SyncMode::Validate => "validate",
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

#[derive(Default)]
struct TableDiffByPath {
    missing: usize,
    stale: usize,
    mismatched: usize,
}

#[derive(Default)]
struct TableDiff {
    missing: usize,
    stale: usize,
    mismatched: usize,
    by_path: std::collections::HashMap<String, TableDiffByPath>,
}

struct TempSqliteCleanup {
    path: PathBuf,
}

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) async fn execute_sync_validation(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<SyncSummary> {
    let temp_parent = std::env::temp_dir().join("bitloops").join("sync-validate");
    fs::create_dir_all(&temp_parent).with_context(|| {
        format!(
            "creating temporary sync validation directory at {}",
            temp_parent.display()
        )
    })?;
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("computing sync validation run identifier")?
        .as_nanos();
    let sqlite_path = temp_parent.join(format!(
        "sync_validate_{}_{}.sqlite",
        std::process::id(),
        run_id
    ));
    let _cleanup = TempSqliteCleanup {
        path: sqlite_path.clone(),
    };
    init_sqlite_schema(&sqlite_path)
        .await
        .context("initialising temporary SQLite schema for sync validation")?;
    let expected_store = RelationalStorage::local_only(sqlite_path);
    let expected_projection =
        execute_sync(cfg, &expected_store, sync::types::SyncMode::Full).await?;

    let expected_artefacts = load_artefact_rows(&expected_store, &cfg.repo.repo_id).await?;
    let actual_artefacts = load_artefact_rows(relational, &cfg.repo.repo_id).await?;
    let expected_edges = load_edge_rows(&expected_store, &cfg.repo.repo_id).await?;
    let actual_edges = load_edge_rows(relational, &cfg.repo.repo_id).await?;

    let artefact_diff = compare_rows_by_key(
        &expected_artefacts,
        &actual_artefacts,
        &["path", "symbol_id"],
    );
    let edge_diff = compare_rows_by_key(&expected_edges, &actual_edges, &["path", "edge_id"]);
    let files_with_drift = merge_file_drift(&artefact_diff, &edge_diff);

    let validation = SyncValidationSummary {
        valid: artefact_diff.missing == 0
            && artefact_diff.stale == 0
            && artefact_diff.mismatched == 0
            && edge_diff.missing == 0
            && edge_diff.stale == 0
            && edge_diff.mismatched == 0,
        expected_artefacts: expected_artefacts.len(),
        actual_artefacts: actual_artefacts.len(),
        expected_edges: expected_edges.len(),
        actual_edges: actual_edges.len(),
        missing_artefacts: artefact_diff.missing,
        stale_artefacts: artefact_diff.stale,
        mismatched_artefacts: artefact_diff.mismatched,
        missing_edges: edge_diff.missing,
        stale_edges: edge_diff.stale,
        mismatched_edges: edge_diff.mismatched,
        files_with_drift,
    };

    Ok(SyncSummary {
        success: validation.valid,
        mode: "validate".to_string(),
        parser_version: expected_projection.parser_version,
        extractor_version: expected_projection.extractor_version,
        active_branch: expected_projection.active_branch,
        head_commit_sha: expected_projection.head_commit_sha,
        head_tree_sha: expected_projection.head_tree_sha,
        paths_unchanged: expected_projection.paths_unchanged,
        paths_added: expected_projection.paths_added,
        paths_changed: expected_projection.paths_changed,
        paths_removed: expected_projection.paths_removed,
        cache_hits: expected_projection.cache_hits,
        cache_misses: expected_projection.cache_misses,
        parse_errors: expected_projection.parse_errors,
        validation: Some(validation),
    })
}

async fn load_artefact_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring \
             FROM artefacts_current \
             WHERE repo_id = '{}' \
             ORDER BY path, symbol_id",
            esc_pg(repo_id),
        ))
        .await?;
    rows_to_objects(rows, "artefacts_current")
}

async fn load_edge_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, edge_id, path, content_id, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
             FROM artefact_edges_current \
             WHERE repo_id = '{}' \
             ORDER BY path, edge_id",
            esc_pg(repo_id),
        ))
        .await?;
    rows_to_objects(rows, "artefact_edges_current")
}

fn rows_to_objects(
    rows: Vec<Value>,
    table_name: &str,
) -> Result<Vec<serde_json::Map<String, Value>>> {
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            row.as_object().cloned().ok_or_else(|| {
                anyhow!(
                    "expected object row from `{table_name}` at index {index}, got {}",
                    row
                )
            })
        })
        .collect()
}

fn compare_rows_by_key(
    expected_rows: &[serde_json::Map<String, Value>],
    actual_rows: &[serde_json::Map<String, Value>],
    key_columns: &[&str],
) -> TableDiff {
    let expected = expected_rows
        .iter()
        .filter_map(|row| row_key(row, key_columns).map(|key| (key, row.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    let actual = actual_rows
        .iter()
        .filter_map(|row| row_key(row, key_columns).map(|key| (key, row.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    let mut diff = TableDiff::default();

    for (key, expected_row) in &expected {
        match actual.get(key) {
            None => {
                diff.missing += 1;
                diff.by_path
                    .entry(row_path(expected_row))
                    .or_default()
                    .missing += 1;
            }
            Some(actual_row) if expected_row != actual_row => {
                diff.mismatched += 1;
                diff.by_path
                    .entry(row_path(expected_row))
                    .or_default()
                    .mismatched += 1;
            }
            _ => {}
        }
    }

    for (key, actual_row) in &actual {
        if !expected.contains_key(key) {
            diff.stale += 1;
            diff.by_path.entry(row_path(actual_row)).or_default().stale += 1;
        }
    }

    diff
}

fn row_key(row: &serde_json::Map<String, Value>, key_columns: &[&str]) -> Option<String> {
    let mut values = Vec::with_capacity(key_columns.len());
    for column in key_columns {
        values.push(row.get(*column)?.to_string());
    }
    Some(values.join("|"))
}

fn row_path(row: &serde_json::Map<String, Value>) -> String {
    row.get("path")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>")
        .to_string()
}

fn merge_file_drift(
    artefact_diff: &TableDiff,
    edge_diff: &TableDiff,
) -> Vec<SyncValidationFileDrift> {
    let mut files = std::collections::HashMap::<String, SyncValidationFileDrift>::new();

    for (path, counts) in &artefact_diff.by_path {
        let entry = files
            .entry(path.clone())
            .or_insert_with(|| SyncValidationFileDrift {
                path: path.clone(),
                ..SyncValidationFileDrift::default()
            });
        entry.missing_artefacts += counts.missing;
        entry.stale_artefacts += counts.stale;
        entry.mismatched_artefacts += counts.mismatched;
    }

    for (path, counts) in &edge_diff.by_path {
        let entry = files
            .entry(path.clone())
            .or_insert_with(|| SyncValidationFileDrift {
                path: path.clone(),
                ..SyncValidationFileDrift::default()
            });
        entry.missing_edges += counts.missing;
        entry.stale_edges += counts.stale;
        entry.mismatched_edges += counts.mismatched;
    }

    let mut drift = files
        .into_values()
        .filter(|file| {
            file.missing_artefacts > 0
                || file.stale_artefacts > 0
                || file.mismatched_artefacts > 0
                || file.missing_edges > 0
                || file.stale_edges > 0
                || file.mismatched_edges > 0
        })
        .collect::<Vec<_>>();
    drift.sort_by(|left, right| left.path.cmp(&right.path));
    drift
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

fn is_missing_sync_schema_error(err: &anyhow::Error) -> bool {
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

    #[test]
    fn compare_rows_by_key_reports_missing_stale_and_mismatched_by_path() {
        let expected = vec![
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "a",
                "kind": "function"
            })
            .as_object()
            .expect("object")
            .clone(),
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "b",
                "kind": "module"
            })
            .as_object()
            .expect("object")
            .clone(),
        ];
        let actual = vec![
            serde_json::json!({
                "path": "src/lib.rs",
                "symbol_id": "a",
                "kind": "module"
            })
            .as_object()
            .expect("object")
            .clone(),
            serde_json::json!({
                "path": "src/main.rs",
                "symbol_id": "x",
                "kind": "function"
            })
            .as_object()
            .expect("object")
            .clone(),
        ];

        let diff = compare_rows_by_key(&expected, &actual, &["path", "symbol_id"]);
        assert_eq!(diff.missing, 1);
        assert_eq!(diff.stale, 1);
        assert_eq!(diff.mismatched, 1);
        assert_eq!(diff.by_path.get("src/lib.rs").map(|d| d.missing), Some(1));
        assert_eq!(
            diff.by_path.get("src/lib.rs").map(|d| d.mismatched),
            Some(1)
        );
        assert_eq!(diff.by_path.get("src/main.rs").map(|d| d.stale), Some(1));
    }
}
