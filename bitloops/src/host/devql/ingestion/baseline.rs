use super::*;

const BASELINE_BATCH_SIZE: usize = 200;
const BASELINE_SYNC_STATE_KEY: &str = "baseline_commit_sha";

pub(super) fn discover_baseline_files(repo_root: &Path) -> Result<Vec<String>> {
    let tree_output = match run_git(repo_root, &["ls-tree", "-r", "--full-tree", "HEAD"]) {
        Ok(output) => output,
        Err(err) if is_missing_head_error(&err) => return Ok(Vec::new()),
        Err(err) => return Err(err).context("listing tracked files at HEAD"),
    };

    let mut files = tree_output
        .lines()
        .filter_map(|line| {
            let (meta, raw_path) = line.split_once('\t')?;
            let mut meta_parts = meta.split_whitespace();
            let _mode = meta_parts.next()?;
            let object_type = meta_parts.next()?;
            if object_type != "blob" {
                return None;
            }
            let normalized_path = normalize_repo_path(raw_path);
            if normalized_path.is_empty() || !is_supported_baseline_file(&normalized_path) {
                return None;
            }
            Some(normalized_path)
        })
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(files)
}

fn is_supported_baseline_file(path: &str) -> bool {
    let Some(extension) = Path::new(path).extension().and_then(|value| value.to_str()) else {
        return false;
    };

    matches!(
        extension.trim().to_ascii_lowercase().as_str(),
        "rs" | "ts" | "tsx" | "js" | "jsx"
    )
}

pub(super) async fn run_baseline_ingestion(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<()> {
    let head_sha = match run_git(&cfg.repo_root, &["rev-parse", "HEAD"]) {
        Ok(sha) => sha,
        Err(err) if is_missing_head_error(&err) => {
            println!("Baseline ingestion skipped: repository has no commits yet.");
            return Ok(());
        }
        Err(err) => return Err(err).context("resolving HEAD for baseline ingestion"),
    };
    let branch = active_branch_name(&cfg.repo_root);
    let files = discover_baseline_files(&cfg.repo_root)?;
    let previous_baseline_sha =
        load_sync_state_value(cfg, relational, BASELINE_SYNC_STATE_KEY).await?;
    let current_branch_rows = count_current_branch_file_rows(cfg, relational, &branch).await?;

    if previous_baseline_sha.as_deref() == Some(head_sha.as_str())
        && (current_branch_rows > 0 || files.is_empty())
    {
        println!("Baseline ingestion skipped: active branch is already indexed at HEAD.");
        return Ok(());
    }

    ensure_repository_row(cfg, relational).await?;
    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, &head_sha).unwrap_or(
        CheckpointCommitInfo {
            commit_sha: head_sha.clone(),
            commit_unix: 0,
            author_name: String::new(),
            author_email: String::new(),
            subject: String::new(),
        },
    );
    upsert_commit_metadata_row(cfg, relational, &commit_info).await?;

    if files.is_empty() {
        upsert_sync_state_value(cfg, relational, BASELINE_SYNC_STATE_KEY, &head_sha).await?;
        println!("Baseline ingestion skipped: no supported source files at HEAD.");
        return Ok(());
    }

    println!("Indexing codebase ({} files)...", files.len());
    let mut processed = 0usize;
    for chunk in files.chunks(BASELINE_BATCH_SIZE) {
        for path in chunk {
            let Some(blob_sha) = git_blob_sha_at_commit(&cfg.repo_root, &head_sha, path) else {
                continue;
            };

            upsert_file_state_row(&cfg.repo.repo_id, relational, &head_sha, path, &blob_sha)
                .await?;
            let file_artefact = upsert_file_artefact_row(
                &cfg.repo.repo_id,
                &cfg.repo_root,
                relational,
                path,
                &blob_sha,
            )
            .await?;
            upsert_language_artefacts(
                cfg,
                relational,
                &FileRevision {
                    commit_sha: &head_sha,
                    revision: TemporalRevisionRef {
                        kind: TemporalRevisionKind::Commit,
                        id: &head_sha,
                        temp_checkpoint_id: None,
                    },
                    commit_unix: commit_info.commit_unix,
                    path,
                    blob_sha: &blob_sha,
                },
                &file_artefact,
            )
            .await?;
            processed += 1;
        }
        println!("{}", render_baseline_progress(processed, files.len()));
    }

    let tracked_paths = files.iter().cloned().collect::<HashSet<_>>();
    cleanup_removed_branch_paths(cfg, relational, &branch, &tracked_paths).await?;
    upsert_sync_state_value(cfg, relational, BASELINE_SYNC_STATE_KEY, &head_sha).await?;

    let artefacts_count =
        count_current_branch_rows(cfg, relational, &branch, "artefacts_current").await?;
    let edges_count =
        count_current_branch_rows(cfg, relational, &branch, "artefact_edges_current").await?;
    println!(
        "Baseline ingestion complete: {} files, {} artefacts, {} edges",
        processed, artefacts_count, edges_count
    );
    Ok(())
}

fn render_baseline_progress(processed: usize, total: usize) -> String {
    if total == 0 {
        return "[------------------------------] 0/0 files (100%)".to_string();
    }

    let bar_width = 30usize;
    let filled = (processed.saturating_mul(bar_width) + (total / 2)) / total;
    let percent = (processed.saturating_mul(100) + (total / 2)) / total;
    format!(
        "[{}{}] {}/{} files ({}%)",
        "=".repeat(filled.min(bar_width)),
        "-".repeat(bar_width.saturating_sub(filled.min(bar_width))),
        processed.min(total),
        total,
        percent.min(100),
    )
}

async fn cleanup_removed_branch_paths(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    tracked_paths: &HashSet<String>,
) -> Result<()> {
    let sql = format!(
        "SELECT path FROM artefacts_current \
WHERE repo_id = '{}' AND branch = '{}' AND canonical_kind = 'file'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
    );
    let rows = relational.query_rows(&sql).await?;
    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !tracked_paths.contains(path) {
            delete_current_state_for_path(cfg, relational, path).await?;
        }
    }
    Ok(())
}

async fn load_sync_state_value(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    key: &str,
) -> Result<Option<String>> {
    let sql = format!(
        "SELECT state_value FROM sync_state WHERE repo_id = '{}' AND state_key = '{}' LIMIT 1",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(key),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("state_value"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

fn build_upsert_sync_state_sql(
    repo_id: &str,
    key: &str,
    value: &str,
    relational: &RelationalStorage,
) -> String {
    let now_sql = sql_now(relational);
    format!(
        "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) VALUES ('{}', '{}', '{}', {}) \
ON CONFLICT (repo_id, state_key) DO UPDATE SET state_value = EXCLUDED.state_value, updated_at = {}",
        esc_pg(repo_id),
        esc_pg(key),
        esc_pg(value),
        now_sql,
        now_sql,
    )
}

async fn upsert_sync_state_value(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    key: &str,
    value: &str,
) -> Result<()> {
    let sql = build_upsert_sync_state_sql(&cfg.repo.repo_id, key, value, relational);
    relational.exec(&sql).await
}

async fn count_current_branch_file_rows(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) AS row_count FROM artefacts_current \
WHERE repo_id = '{}' AND branch = '{}' AND canonical_kind = 'file'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
    );
    count_rows(relational, &sql).await
}

async fn count_current_branch_rows(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
    table_name: &str,
) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) AS row_count FROM {} WHERE repo_id = '{}' AND branch = '{}'",
        table_name,
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
    );
    count_rows(relational, &sql).await
}

async fn count_rows(relational: &RelationalStorage, sql: &str) -> Result<usize> {
    let rows = relational.query_rows(sql).await?;
    let count = rows
        .first()
        .and_then(|row| row.get("row_count"))
        .and_then(|value| {
            value
                .as_u64()
                .map(|number| number as usize)
                .or_else(|| value.as_i64().map(|number| number.max(0) as usize))
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<usize>().ok()))
        })
        .unwrap_or_default();
    Ok(count)
}

pub(super) async fn upsert_commit_metadata_row(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_info: &CheckpointCommitInfo,
) -> Result<()> {
    let committed_at_sql = match relational.dialect() {
        RelationalDialect::Postgres => format!("to_timestamp({})", commit_info.commit_unix),
        RelationalDialect::Sqlite => format!("datetime({}, 'unixepoch')", commit_info.commit_unix),
    };
    let sql = format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) \
VALUES ('{}', '{}', '{}', '{}', '{}', {}) \
ON CONFLICT (commit_sha) DO UPDATE SET repo_id = EXCLUDED.repo_id, author_name = EXCLUDED.author_name, author_email = EXCLUDED.author_email, commit_message = EXCLUDED.commit_message, committed_at = EXCLUDED.committed_at",
        esc_pg(&commit_info.commit_sha),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&commit_info.author_name),
        esc_pg(&commit_info.author_email),
        esc_pg(&commit_info.subject),
        committed_at_sql,
    );
    relational.exec(&sql).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_baseline_file_extensions_are_whitelisted() {
        assert!(is_supported_baseline_file("src/lib.rs"));
        assert!(is_supported_baseline_file("src/main.ts"));
        assert!(is_supported_baseline_file("src/main.tsx"));
        assert!(is_supported_baseline_file("src/main.js"));
        assert!(is_supported_baseline_file("src/main.jsx"));
        assert!(!is_supported_baseline_file("README.md"));
        assert!(!is_supported_baseline_file("src/main.py"));
    }
}
