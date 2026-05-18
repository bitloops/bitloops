use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

#[cfg(test)]
pub(crate) fn capture_temporary_checkpoint_batch(
    cfg: &crate::host::devql::DevqlConfig,
    changed_paths: &[PathBuf],
) -> Result<()> {
    let Some(changes) = prepare_capture_temporary_checkpoint_batch(cfg, changed_paths)? else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating watcher capture runtime")?;
    match runtime.block_on(sync_changed_paths(cfg, &changes.modified, &changes.deleted))? {
        SyncChangedPathsOutcome::Completed => persist_workspace_revision(cfg, &changes.tree_hash),
        SyncChangedPathsOutcome::SkippedByPolicy => Ok(()),
    }
}

pub(crate) fn capture_temporary_checkpoint_batch_with_handle(
    cfg: &crate::host::devql::DevqlConfig,
    changed_paths: &[PathBuf],
    handle: &tokio::runtime::Handle,
) -> Result<()> {
    let Some(changes) = prepare_capture_temporary_checkpoint_batch(cfg, changed_paths)? else {
        return Ok(());
    };
    match handle.block_on(sync_changed_paths(cfg, &changes.modified, &changes.deleted))? {
        SyncChangedPathsOutcome::Completed => persist_workspace_revision(cfg, &changes.tree_hash),
        SyncChangedPathsOutcome::SkippedByPolicy => Ok(()),
    }
}

struct PreparedCaptureBatch {
    modified: Vec<String>,
    deleted: Vec<String>,
    tree_hash: String,
}

enum SyncChangedPathsOutcome {
    Completed,
    SkippedByPolicy,
}

fn prepare_capture_temporary_checkpoint_batch(
    cfg: &crate::host::devql::DevqlConfig,
    changed_paths: &[PathBuf],
) -> Result<Option<PreparedCaptureBatch>> {
    if changed_paths.is_empty() {
        return Ok(None);
    }

    let repo_root = &cfg.repo_root;
    let base_commit = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", "HEAD"],
    )
    .unwrap_or_default();

    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for path in changed_paths {
        let rel = path.strip_prefix(repo_root).unwrap_or(path.as_path());
        let rel = rel.to_string_lossy().to_string();
        if rel.is_empty() {
            continue;
        }
        if path.exists() {
            if path.is_file() {
                modified.push(rel);
            }
        } else {
            deleted.push(rel);
        }
    }

    if modified.is_empty() && deleted.is_empty() {
        return Ok(None);
    }

    let parent_tree = crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["rev-parse", &format!("{}^{{tree}}", base_commit)],
    )
    .ok();

    let tree_hash = crate::host::checkpoints::strategy::manual_commit::build_tree(
        repo_root,
        parent_tree.as_deref(),
        &modified,
        &[],
        &deleted,
    )
    .context("building temporary checkpoint tree hash for devql watch")?;

    let tree_matches_parent = parent_tree.as_deref() == Some(tree_hash.as_str());
    if tree_matches_parent && !local_sqlite_exists_for_capture(cfg)? {
        return Ok(None);
    }

    let latest_tree_hash = latest_workspace_revision_tree_hash(cfg)?;
    if latest_tree_hash.as_deref() == Some(tree_hash.as_str()) {
        return Ok(None);
    }
    if tree_matches_parent && latest_tree_hash.is_none() {
        return Ok(None);
    }

    Ok(Some(PreparedCaptureBatch {
        modified,
        deleted,
        tree_hash,
    }))
}

fn local_sqlite_exists_for_capture(cfg: &crate::host::devql::DevqlConfig) -> Result<bool> {
    let relational = DefaultRelationalStore::open_local_for_repo_root(&cfg.repo_root)
        .context("resolving local relational store for watcher capture")?;
    Ok(relational.sqlite_path().exists())
}

fn latest_workspace_revision_tree_hash(
    cfg: &crate::host::devql::DevqlConfig,
) -> Result<Option<String>> {
    let relational = DefaultRelationalStore::open_local_for_repo_root(&cfg.repo_root)
        .context("opening local relational store for watcher capture")?;
    relational.initialise_local_devql_schema()?;
    let sqlite = RelationalStore::local_sqlite_pool(&relational)?;
    sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT tree_hash FROM workspace_revisions WHERE repo_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![cfg.repo.repo_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

async fn sync_changed_paths(
    cfg: &crate::host::devql::DevqlConfig,
    modified: &[String],
    deleted: &[String],
) -> Result<SyncChangedPathsOutcome> {
    if !crate::config::settings::devql_sync_enabled(&cfg.repo_root)
        .context("loading DevQL sync producer policy for watcher capture")?
    {
        return Ok(SyncChangedPathsOutcome::SkippedByPolicy);
    }

    let mut paths = modified
        .iter()
        .chain(deleted.iter())
        .map(|path| crate::host::devql::normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let paths =
        filter_paths_for_sync(cfg, &paths).context("classifying watcher capture paths for sync")?;
    if paths.is_empty() {
        return Ok(SyncChangedPathsOutcome::Completed);
    }

    #[cfg(test)]
    {
        crate::host::devql::run_sync_with_summary(cfg, crate::host::devql::SyncMode::Paths(paths))
            .await
            .context("running DevQL sync inline for watcher capture paths in tests")?;
        Ok(SyncChangedPathsOutcome::Completed)
    }

    #[cfg(not(test))]
    {
        enqueue_spooled_sync_task_with_retry(cfg, paths)
            .context("queueing DevQL sync for watcher capture paths in repo-local spool")?;
        Ok(SyncChangedPathsOutcome::Completed)
    }
}

#[cfg(not(test))]
fn enqueue_spooled_sync_task_with_retry(
    cfg: &crate::host::devql::DevqlConfig,
    paths: Vec<String>,
) -> Result<()> {
    const MAX_ATTEMPTS: usize = 5;
    for attempt in 1..=MAX_ATTEMPTS {
        match crate::host::devql::enqueue_spooled_sync_task(
            cfg,
            crate::daemon::DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(paths.clone()),
        ) {
            Ok(_) => return Ok(()),
            Err(err) => {
                if attempt == MAX_ATTEMPTS
                    || !err
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("database is locked")
                {
                    return Err(err);
                }
                std::thread::sleep(std::time::Duration::from_millis(100_u64 * attempt as u64));
            }
        }
    }
    Ok(())
}

fn persist_workspace_revision(
    cfg: &crate::host::devql::DevqlConfig,
    tree_hash: &str,
) -> Result<()> {
    let relational = DefaultRelationalStore::open_local_for_repo_root(&cfg.repo_root)
        .context("opening local relational store for watcher workspace revision persist")?;
    relational.initialise_local_devql_schema()?;
    let sqlite = RelationalStore::local_sqlite_pool(&relational)?;
    sqlite.with_write_connection(|conn| {
        let tx = conn
            .unchecked_transaction()
            .context("starting watcher workspace revision transaction")?;
        tx.execute(
            "DELETE FROM workspace_revisions WHERE repo_id = ?1 AND tree_hash = ?2",
            rusqlite::params![cfg.repo.repo_id.as_str(), tree_hash],
        )
        .context("removing prior watcher workspace revision for tree hash")?;
        tx.execute(
            "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES (?1, ?2)",
            rusqlite::params![cfg.repo.repo_id.as_str(), tree_hash],
        )
        .context("inserting latest watcher workspace revision")?;
        tx.commit()
            .context("committing watcher workspace revision transaction")?;
        Ok(())
    })
}

fn filter_paths_for_sync(
    cfg: &crate::host::devql::DevqlConfig,
    paths: &[String],
) -> Result<Vec<String>> {
    let exclusion_matcher = crate::host::devql::load_repo_exclusion_matcher(&cfg.repo_root)
        .context("loading repo policy exclusions for watcher capture sync")?;
    let (parser_version, extractor_version) = resolve_pack_versions_for_capture()
        .context("resolving language pack versions for watcher capture sync")?;
    let classifier = crate::host::devql::ProjectAwareClassifier::discover_for_worktree(
        &cfg.repo_root,
        paths.iter().map(String::as_str),
        &parser_version,
        &extractor_version,
    )
    .context("building project-aware classifier for watcher capture sync")?;
    let mut filtered = Vec::new();
    for path in paths {
        let classification = classifier
            .classify_repo_relative_path(path, exclusion_matcher.excludes_repo_relative_path(path))
            .with_context(|| format!("classifying watcher capture path `{path}`"))?;
        if classification.analysis_mode != crate::host::devql::AnalysisMode::Excluded {
            filtered.push(path.clone());
        }
    }
    Ok(filtered)
}

fn resolve_pack_versions_for_capture() -> Result<(String, String)> {
    let host = crate::host::devql::core_extension_host()?;
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

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
