use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::Connection;

use super::diff_collector::DiffArtefactRecord;
use super::shared::{determine_retention_class, read_effective_content};
use super::stats::PreparedPathStats;
use crate::host::devql::sync::content_cache::{
    CacheKey, CachedExtraction, deduped_cached_content_parts,
    lookup_cached_content_with_connection, persist_cached_content_tx, touch_cache_entries_tx,
};
use crate::host::devql::sync::materializer::{
    PreparedMaterialisationRows, persist_prepared_materialisation_tx, prepare_materialization_rows,
    remove_paths_tx, resolve_prepared_local_edges_with_connection,
};
use crate::host::devql::sync::types::DesiredFileState;
use crate::host::devql::{DecodedFileContent, DevqlConfig};

const BATCH_FILE_LIMIT: usize = 32;
const BATCH_ROW_LIMIT: usize = 4000;
const BATCH_MAX_AGE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub(crate) struct PreparedRemoval {
    pub(crate) path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedSyncItem {
    pub(crate) index: usize,
    pub(crate) desired: DesiredFileState,
    pub(crate) extraction: CachedExtraction,
    pub(crate) prepared_rows: PreparedMaterialisationRows,
    pub(crate) cache_store_retention_class: Option<&'static str>,
    pub(crate) cache_touch_key: Option<CacheKey>,
    pub(crate) promote_cache_entry_to_git_backed: bool,
}

impl PreparedSyncItem {
    fn row_operation_estimate(&self) -> usize {
        let mut operations = self.prepared_rows.row_operation_estimate();
        if self.cache_store_retention_class.is_some() {
            let (artefacts, edges) = deduped_cached_content_parts(&self.extraction);
            operations += 3 + artefacts.len() + edges.len();
        }
        operations
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedSyncOutcome {
    pub(crate) path: String,
    pub(crate) prepared_item: Option<PreparedSyncItem>,
    pub(crate) cache_hit: bool,
    pub(crate) cache_miss: bool,
    pub(crate) parse_error: bool,
    pub(crate) error_message: Option<String>,
    pub(crate) stats: PreparedPathStats,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SyncBatch {
    items: Vec<PreparedSyncItem>,
    row_operation_estimate: usize,
    first_item_at: Option<Instant>,
}

impl SyncBatch {
    fn push(&mut self, item: PreparedSyncItem) {
        if self.first_item_at.is_none() {
            self.first_item_at = Some(Instant::now());
        }
        self.row_operation_estimate += item.row_operation_estimate();
        self.items.push(item);
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn should_flush(&self) -> bool {
        if self.items.len() >= BATCH_FILE_LIMIT || self.row_operation_estimate >= BATCH_ROW_LIMIT {
            return true;
        }
        self.first_item_at
            .is_some_and(|started| started.elapsed() >= BATCH_MAX_AGE)
    }

    fn flush_deadline(&self) -> Option<Instant> {
        self.first_item_at.map(|started| started + BATCH_MAX_AGE)
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct WriterCommitOutcome {
    pub(crate) materialized_paths: Vec<String>,
    pub(crate) removed_paths: Vec<String>,
    pub(crate) pre_artefacts: Vec<DiffArtefactRecord>,
    pub(crate) post_artefacts: Vec<DiffArtefactRecord>,
    pub(crate) sqlite_commits: usize,
    pub(crate) sqlite_rows_written: usize,
    pub(crate) cache_store_operation_estimate: usize,
    pub(crate) materialisation_operation_estimate: usize,
}

#[derive(Clone)]
pub(crate) struct SqliteReadConnectionPool {
    connections: Arc<Mutex<Vec<Connection>>>,
}

impl SqliteReadConnectionPool {
    pub(crate) async fn open(path: &Path, size: usize) -> Result<Self> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut connections = Vec::with_capacity(size);
            for _ in 0..size {
                connections.push(open_sync_sqlite_connection(&path)?);
            }
            Ok(Self {
                connections: Arc::new(Mutex::new(connections)),
            })
        })
        .await
        .context("joining SQLite read connection pool task")?
    }

    fn checkout(&self) -> Result<PooledSqliteConnection> {
        let mut connections = self
            .connections
            .lock()
            .map_err(|_| anyhow!("locking SQLite read connection pool"))?;
        let connection = connections
            .pop()
            .ok_or_else(|| anyhow!("SQLite read connection pool exhausted"))?;
        Ok(PooledSqliteConnection {
            pool: Arc::clone(&self.connections),
            connection: Some(connection),
        })
    }
}

struct PooledSqliteConnection {
    pool: Arc<Mutex<Vec<Connection>>>,
    connection: Option<Connection>,
}

impl PooledSqliteConnection {
    fn connection(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("pooled SQLite connection should be present")
    }
}

impl Drop for PooledSqliteConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        if let Ok(mut pool) = self.pool.lock() {
            pool.push(connection);
        }
    }
}

pub(crate) struct SqliteSyncWriter {
    connection: Arc<Mutex<Connection>>,
    pending_batch: SyncBatch,
    pending_touches: HashMap<CacheKey, bool>,
}

impl SqliteSyncWriter {
    pub(crate) async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let connection = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = open_sync_sqlite_connection(&path)?;
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
                .context("configuring SQLite writer connection")?;
            Ok(conn)
        })
        .await
        .context("joining SQLite writer connection task")??;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            pending_batch: SyncBatch::default(),
            pending_touches: HashMap::new(),
        })
    }

    pub(crate) fn push_item(&mut self, item: PreparedSyncItem) {
        if let Some(key) = item.cache_touch_key.clone() {
            self.pending_touches
                .entry(key)
                .and_modify(|existing| *existing |= item.promote_cache_entry_to_git_backed)
                .or_insert(item.promote_cache_entry_to_git_backed);
        }
        self.pending_batch.push(item);
    }

    pub(crate) fn has_pending_items(&self) -> bool {
        !self.pending_batch.is_empty()
    }

    pub(crate) fn flush_deadline(&self) -> Option<Instant> {
        self.pending_batch.flush_deadline()
    }

    pub(crate) fn should_flush(&self) -> bool {
        self.pending_batch.should_flush()
    }

    pub(crate) async fn flush(
        &mut self,
        repo_id: &str,
        parser_version: &str,
        extractor_version: &str,
    ) -> Result<WriterCommitOutcome> {
        if self.pending_batch.is_empty() {
            return Ok(WriterCommitOutcome::default());
        }

        let mut batch = std::mem::take(&mut self.pending_batch);
        batch.items.sort_by_key(|item| item.index);
        let connection = Arc::clone(&self.connection);
        let repo_id = repo_id.to_string();
        let parser_version = parser_version.to_string();
        let extractor_version = extractor_version.to_string();

        tokio::task::spawn_blocking(move || -> Result<WriterCommitOutcome> {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("locking SQLite sync writer connection"))?;
            let tx = connection
                .transaction()
                .context("starting SQLite sync writer transaction")?;
            let mut rows_written = 0usize;
            let mut cache_store_operation_estimate = 0usize;
            let mut materialisation_operation_estimate = 0usize;
            let mut pre_artefacts = Vec::new();
            let mut post_artefacts = Vec::new();
            let materialized_paths = batch
                .items
                .iter()
                .map(|item| item.desired.path.clone())
                .collect::<Vec<_>>();

            for item in &batch.items {
                if let Some(retention_class) = item.cache_store_retention_class {
                    let (artefacts, edges) = deduped_cached_content_parts(&item.extraction);
                    cache_store_operation_estimate += 3 + artefacts.len() + edges.len();
                    rows_written +=
                        persist_cached_content_tx(&tx, &item.extraction, retention_class)
                            .with_context(|| {
                                format!("persisting cached extraction for `{}`", item.desired.path)
                            })?;
                }
                pre_artefacts.extend(query_current_artefacts_for_path_tx(
                    &tx,
                    &repo_id,
                    &item.desired.path,
                )?);
                materialisation_operation_estimate += item.prepared_rows.row_operation_estimate();
                rows_written += persist_prepared_materialisation_tx(
                    &tx,
                    &repo_id,
                    &item.desired,
                    &item.extraction,
                    &item.prepared_rows,
                    &parser_version,
                    &extractor_version,
                )
                .with_context(|| {
                    format!(
                        "materialising `{}` in SQLite sync writer",
                        item.desired.path
                    )
                })?;
                post_artefacts.extend(query_current_artefacts_for_path_tx(
                    &tx,
                    &repo_id,
                    &item.desired.path,
                )?);
            }

            tx.commit()
                .context("committing SQLite sync writer transaction")?;
            Ok(WriterCommitOutcome {
                materialized_paths,
                removed_paths: Vec::new(),
                pre_artefacts,
                post_artefacts,
                sqlite_commits: 1,
                sqlite_rows_written: rows_written,
                cache_store_operation_estimate,
                materialisation_operation_estimate,
            })
        })
        .await
        .context("joining SQLite sync writer flush task")?
    }

    pub(crate) async fn finish(&mut self) -> Result<WriterCommitOutcome> {
        if self.pending_touches.is_empty() {
            return Ok(WriterCommitOutcome::default());
        }

        let touches = std::mem::take(&mut self.pending_touches);
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<WriterCommitOutcome> {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("locking SQLite sync writer connection"))?;
            let tx = connection
                .transaction()
                .context("starting SQLite sync cache touch transaction")?;
            let rows_written =
                touch_cache_entries_tx(&tx, &touches).context("touching cached sync entries")?;
            tx.commit()
                .context("committing SQLite sync cache touch transaction")?;
            Ok(WriterCommitOutcome {
                materialized_paths: Vec::new(),
                removed_paths: Vec::new(),
                pre_artefacts: Vec::new(),
                post_artefacts: Vec::new(),
                sqlite_commits: 1,
                sqlite_rows_written: rows_written,
                cache_store_operation_estimate: rows_written,
                materialisation_operation_estimate: 0,
            })
        })
        .await
        .context("joining SQLite sync cache touch task")?
    }

    pub(crate) async fn remove_paths(
        &mut self,
        repo_id: &str,
        removals: &[PreparedRemoval],
    ) -> Result<WriterCommitOutcome> {
        if removals.is_empty() {
            return Ok(WriterCommitOutcome::default());
        }

        let repo_id = repo_id.to_string();
        let removed_paths = removals
            .iter()
            .map(|removal| removal.path.clone())
            .collect::<Vec<_>>();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<WriterCommitOutcome> {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("locking SQLite sync writer connection"))?;
            let tx = connection
                .transaction()
                .context("starting SQLite removal transaction")?;
            let mut pre_artefacts = Vec::new();
            for path in &removed_paths {
                pre_artefacts.extend(query_current_artefacts_for_path_tx(&tx, &repo_id, path)?);
            }
            let rows_written = remove_paths_tx(&tx, &repo_id, &removed_paths)
                .context("removing deleted sync paths")?;
            tx.commit()
                .context("committing SQLite removal transaction")?;
            Ok(WriterCommitOutcome {
                materialized_paths: Vec::new(),
                removed_paths,
                pre_artefacts,
                post_artefacts: Vec::new(),
                sqlite_commits: 1,
                sqlite_rows_written: rows_written,
                cache_store_operation_estimate: 0,
                materialisation_operation_estimate: rows_written,
            })
        })
        .await
        .context("joining SQLite removal task")?
    }

    pub(crate) async fn run_gc(
        &mut self,
        ttl_days: u32,
    ) -> Result<(crate::host::devql::sync::gc::GcResult, WriterCommitOutcome)> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<_> {
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow!("locking SQLite sync writer connection"))?;
            let (result, rows_written) =
                crate::host::devql::sync::gc::run_gc_with_connection(&mut connection, ttl_days)?;
            let outcome = if rows_written == 0 {
                WriterCommitOutcome::default()
            } else {
                WriterCommitOutcome {
                    materialized_paths: Vec::new(),
                    removed_paths: Vec::new(),
                    pre_artefacts: Vec::new(),
                    post_artefacts: Vec::new(),
                    sqlite_commits: 1,
                    sqlite_rows_written: rows_written,
                    cache_store_operation_estimate: rows_written,
                    materialisation_operation_estimate: 0,
                }
            };
            Ok((result, outcome))
        })
        .await
        .context("joining SQLite sync gc task")?
    }
}

pub(crate) async fn prepare_sync_item(
    pool: SqliteReadConnectionPool,
    cfg: Arc<DevqlConfig>,
    desired: DesiredFileState,
    index: usize,
    parser_version: Arc<String>,
    extractor_version: Arc<String>,
) -> PreparedSyncOutcome {
    let path_for_error = desired.path.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let pooled_connection = pool.checkout()?;
        prepare_sync_item_with_connection(
            pooled_connection.connection(),
            cfg.as_ref(),
            desired,
            index,
            parser_version.as_str(),
            extractor_version.as_str(),
        )
    })
    .await;

    match joined {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(err)) => prepare_failure_outcome(path_for_error, format!("{err:#}")),
        Err(err) => prepare_failure_outcome(
            path_for_error,
            format!("joining sync prepare worker task failed: {err}"),
        ),
    }
}

fn prepare_failure_outcome(path: String, error_message: String) -> PreparedSyncOutcome {
    PreparedSyncOutcome {
        path,
        prepared_item: None,
        cache_hit: false,
        cache_miss: true,
        parse_error: true,
        error_message: Some(error_message),
        stats: PreparedPathStats::default(),
    }
}

fn parse_status_counts_as_parse_error(parse_status: &str) -> bool {
    matches!(
        parse_status,
        crate::host::devql::sync::extraction::PARSE_STATUS_PARSE_ERROR
            | crate::host::devql::sync::extraction::PARSE_STATUS_DECODE_ERROR
            | crate::host::devql::sync::extraction::PARSE_STATUS_DEGRADED_FILE_ONLY
    )
}

fn build_code_file_only_fallback_extraction(
    desired: &DesiredFileState,
    content: &DecodedFileContent,
    parser_version: &str,
    extractor_version: &str,
) -> Option<CachedExtraction> {
    if desired.analysis_mode != crate::host::devql::AnalysisMode::Code {
        return None;
    }
    if content.decode_degraded {
        return Some(
            crate::host::devql::sync::extraction::decode_error_file_only_to_cache_format(
                &desired.path,
                &desired.effective_content_id,
                &desired.language,
                &desired.extraction_fingerprint,
                parser_version,
                extractor_version,
                &content.raw_bytes,
            ),
        );
    }
    if content.contains_nul_bytes() {
        return Some(
            crate::host::devql::sync::extraction::degraded_file_only_to_cache_format(
                &desired.path,
                &desired.effective_content_id,
                &desired.language,
                &desired.extraction_fingerprint,
                parser_version,
                extractor_version,
                &content.raw_bytes,
            ),
        );
    }
    None
}

fn prepare_sync_item_with_connection(
    connection: &Connection,
    cfg: &DevqlConfig,
    desired: DesiredFileState,
    index: usize,
    parser_version: &str,
    extractor_version: &str,
) -> Result<PreparedSyncOutcome> {
    let mut stats = PreparedPathStats::default();
    let retention_class = determine_retention_class(&desired);
    let path = desired.path.clone();

    if desired.analysis_mode == crate::host::devql::AnalysisMode::TrackOnly {
        let extraction = CachedExtraction {
            content_id: desired.effective_content_id.clone(),
            language: desired.language.clone(),
            extraction_fingerprint: desired.extraction_fingerprint.clone(),
            parser_version: parser_version.to_string(),
            extractor_version: extractor_version.to_string(),
            parse_status: crate::host::devql::sync::extraction::PARSE_STATUS_OK.to_string(),
            artefacts: Vec::new(),
            edges: Vec::new(),
        };
        let materialisation_prep_started = Instant::now();
        let prepared_rows = prepare_materialization_rows(
            cfg,
            &desired,
            &extraction,
            parser_version,
            extractor_version,
        )?;
        let mut prepared_rows = prepared_rows;
        resolve_prepared_local_edges_with_connection(
            connection,
            cfg,
            &desired,
            &mut prepared_rows,
        )?;
        stats.materialisation_prep = materialisation_prep_started.elapsed();
        return Ok(PreparedSyncOutcome {
            path,
            prepared_item: Some(PreparedSyncItem {
                index,
                desired,
                extraction,
                prepared_rows,
                cache_store_retention_class: None,
                cache_touch_key: None,
                promote_cache_entry_to_git_backed: false,
            }),
            cache_hit: false,
            cache_miss: false,
            parse_error: false,
            error_message: None,
            stats,
        });
    }

    let cache_lookup_started = Instant::now();
    let cached = lookup_cached_content_with_connection(
        connection,
        &desired.effective_content_id,
        &desired.language,
        &desired.extraction_fingerprint,
        parser_version,
        extractor_version,
    )?;
    stats.cache_lookup = cache_lookup_started.elapsed();

    let content = read_effective_content(cfg, &desired)
        .with_context(|| format!("reading effective content for `{}`", desired.path))?;
    let forced_file_only_extraction = build_code_file_only_fallback_extraction(
        &desired,
        &content,
        parser_version,
        extractor_version,
    );

    let (
        extraction,
        cache_hit,
        cache_miss,
        parse_error,
        cache_store_retention_class,
        cache_touch_key,
    ) = match (cached, forced_file_only_extraction) {
        (Some(cached), Some(_))
            if matches!(
                cached.parse_status.as_str(),
                crate::host::devql::sync::extraction::PARSE_STATUS_DECODE_ERROR
                    | crate::host::devql::sync::extraction::PARSE_STATUS_DEGRADED_FILE_ONLY
            ) =>
        {
            let parse_error = parse_status_counts_as_parse_error(&cached.parse_status);
            (
                cached,
                true,
                false,
                parse_error,
                None,
                Some(CacheKey {
                    content_id: desired.effective_content_id.clone(),
                    language: desired.language.clone(),
                    extraction_fingerprint: desired.extraction_fingerprint.clone(),
                    parser_version: parser_version.to_string(),
                    extractor_version: extractor_version.to_string(),
                }),
            )
        }
        (_, Some(extraction)) => (extraction, false, true, true, Some(retention_class), None),
        (Some(cached), None) => {
            let parse_error = parse_status_counts_as_parse_error(&cached.parse_status);
            (
                cached,
                true,
                false,
                parse_error,
                None,
                Some(CacheKey {
                    content_id: desired.effective_content_id.clone(),
                    language: desired.language.clone(),
                    extraction_fingerprint: desired.extraction_fingerprint.clone(),
                    parser_version: parser_version.to_string(),
                    extractor_version: extractor_version.to_string(),
                }),
            )
        }
        (None, None) => {
            let extraction_started = Instant::now();
            let extraction = if let Some(text) = content.text.as_deref() {
                match crate::host::devql::sync::extraction::extract_to_cache_format(
                    cfg,
                    crate::host::devql::sync::extraction::CacheExtractionRequest {
                        path: &desired.path,
                        language: &desired.language,
                        content_id: &desired.effective_content_id,
                        extraction_fingerprint: &desired.extraction_fingerprint,
                        parser_version,
                        extractor_version,
                        content: text,
                    },
                ) {
                    Ok(extraction) => extraction,
                    Err(_err)
                        if desired.analysis_mode == crate::host::devql::AnalysisMode::Code =>
                    {
                        Some(
                            crate::host::devql::sync::extraction::degraded_file_only_to_cache_format(
                                &desired.path,
                                &desired.effective_content_id,
                                &desired.language,
                                &desired.extraction_fingerprint,
                                parser_version,
                                extractor_version,
                                &content.raw_bytes,
                            ),
                        )
                    }
                    Err(err) => {
                        return Err(err)
                            .with_context(|| format!("extracting `{}` into sync cache format", desired.path));
                    }
                }
            } else {
                None
            };
            let Some(extraction) = extraction else {
                stats.extraction = extraction_started.elapsed();
                return Ok(PreparedSyncOutcome {
                    path,
                    prepared_item: None,
                    cache_hit: false,
                    cache_miss: true,
                    parse_error: true,
                    error_message: None,
                    stats,
                });
            };
            stats.extraction = extraction_started.elapsed();
            let parse_error = parse_status_counts_as_parse_error(&extraction.parse_status);
            (
                extraction,
                false,
                true,
                parse_error,
                Some(retention_class),
                None,
            )
        }
    };

    let materialisation_prep_started = Instant::now();
    let prepared_rows = prepare_materialization_rows(
        cfg,
        &desired,
        &extraction,
        parser_version,
        extractor_version,
    )?;
    let mut prepared_rows = prepared_rows;
    resolve_prepared_local_edges_with_connection(connection, cfg, &desired, &mut prepared_rows)?;
    stats.materialisation_prep = materialisation_prep_started.elapsed();

    Ok(PreparedSyncOutcome {
        path,
        prepared_item: Some(PreparedSyncItem {
            index,
            desired,
            extraction,
            prepared_rows,
            cache_store_retention_class,
            cache_touch_key,
            promote_cache_entry_to_git_backed: retention_class == "git_backed",
        }),
        cache_hit,
        cache_miss,
        parse_error,
        error_message: None,
        stats,
    })
}

fn open_sync_sqlite_connection(path: &PathBuf) -> Result<Connection> {
    if !path.is_file() {
        bail!(
            "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
            path.display()
        );
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening SQLite database at {}", path.display()))?;
    connection
        .busy_timeout(Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(connection)
}

fn query_current_artefacts_for_path_tx(
    tx: &rusqlite::Transaction<'_>,
    repo_id: &str,
    path: &str,
) -> Result<Vec<DiffArtefactRecord>> {
    let mut stmt = tx
        .prepare(
            "SELECT artefact_id, symbol_id, canonical_kind, symbol_fqn \
             FROM artefacts_current \
             WHERE repo_id = ?1 AND path = ?2 \
             ORDER BY symbol_id",
        )
        .context("preparing current artefact snapshot query")?;

    let rows = stmt.query_map(rusqlite::params![repo_id, path], |row| {
        let symbol_fqn = row.get::<_, String>(3)?;
        Ok(DiffArtefactRecord::new(
            path,
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            symbol_name_from_fqn(&symbol_fqn),
        ))
    })?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(anyhow::Error::from)
}

fn symbol_name_from_fqn(symbol_fqn: &str) -> String {
    symbol_fqn
        .rsplit("::")
        .next()
        .unwrap_or(symbol_fqn)
        .to_string()
}

pub(crate) fn sync_prepare_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2)
        .clamp(2, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_prepare_worker_count_is_bounded() {
        let count = sync_prepare_worker_count();
        assert!((2..=8).contains(&count), "worker count should be clamped");
    }

    #[test]
    fn open_sync_sqlite_connection_reports_missing_database_file() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("missing.sqlite");
        let err = open_sync_sqlite_connection(&path).expect_err("missing sqlite file should error");
        let message = format!("{err:#}");
        assert!(message.contains("SQLite database file not found"));
        assert!(message.contains("bitloops init"));
    }

    #[test]
    fn sqlite_read_pool_checkout_exhausts_then_recovers_after_drop() {
        let pool = SqliteReadConnectionPool {
            connections: Arc::new(Mutex::new(vec![
                Connection::open_in_memory().expect("open in-memory sqlite"),
            ])),
        };

        let first = pool.checkout().expect("first checkout");
        assert!(
            pool.checkout().is_err(),
            "pool should be exhausted while connection is checked out"
        );
        first
            .connection()
            .execute("CREATE TABLE demo(id INTEGER PRIMARY KEY)", [])
            .expect("execute query on checked-out connection");
        drop(first);

        let second = pool
            .checkout()
            .expect("connection should be returned to pool on drop");
        let value: i64 = second
            .connection()
            .query_row("SELECT 1", [], |row| row.get(0))
            .expect("execute query after connection recycle");
        assert_eq!(value, 1);
    }

    #[test]
    fn sync_batch_default_is_empty_and_has_no_deadline() {
        let batch = SyncBatch::default();
        assert!(batch.is_empty());
        assert!(!batch.should_flush());
        assert!(batch.flush_deadline().is_none());
    }
}
