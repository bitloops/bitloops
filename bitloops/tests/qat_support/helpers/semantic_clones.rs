use crate::qat_support::world::{
    EnrichmentStatusSnapshot, RepresentationKindCounts, SemanticCloneCurrentTableSnapshot,
    SemanticCloneHistoricalTableSnapshot, SemanticCloneProgressObservation,
    SemanticCloneTableSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SemanticCloneStoreEvidence {
    current_artefacts: usize,
    embeddings: usize,
    clone_edges: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticCloneStoreSnapshot {
    path: std::path::PathBuf,
    evidence: SemanticCloneStoreEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneSummaryGroup {
    relation_kind: String,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneSummary {
    total_count: usize,
    groups: Vec<CloneSummaryGroup>,
}

fn semantic_clone_store_evidence_proves_rebuild(
    _clone_edges_metric: Option<u64>,
    evidence: SemanticCloneStoreEvidence,
) -> bool {
    evidence.current_artefacts > 0 && evidence.embeddings > 0 && evidence.clone_edges > 0
}

fn is_missing_table_or_column_error(err: &rusqlite::Error) -> bool {
    let message = err.to_string();
    message.contains("no such table") || message.contains("no such column")
}

fn count_rows_for_repo(
    conn: &rusqlite::Connection,
    table: &str,
    repo_id: &str,
) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE repo_id = ?1");
    match conn.query_row(&sql, rusqlite::params![repo_id], |row| row.get::<_, i64>(0)) {
        Ok(count) => usize::try_from(count)
            .with_context(|| format!("converting `{table}` row count to usize")),
        Err(err) if is_missing_table_or_column_error(&err) => Ok(0),
        Err(err) => Err(err).with_context(|| format!("counting rows in `{table}`")),
    }
}

fn normalize_representation_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "code" | "baseline" | "enriched" => Some("code"),
        "summary" => Some("summary"),
        _ => None,
    }
}

fn load_representation_kind_counts_for_repo(
    conn: &rusqlite::Connection,
    table: &str,
    repo_id: &str,
) -> Result<RepresentationKindCounts> {
    let sql = format!(
        "SELECT representation_kind, COUNT(*) FROM {table} WHERE repo_id = ?1 GROUP BY representation_kind"
    );
    let mut counts = RepresentationKindCounts::default();
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(err) if is_missing_table_or_column_error(&err) => return Ok(counts),
        Err(err) => return Err(err).with_context(|| format!("preparing `{table}` representation query")),
    };
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .with_context(|| format!("querying representation kinds from `{table}`"))?;
    for row in rows {
        let (kind, count) = row.with_context(|| format!("reading `{table}` representation row"))?;
        let count = usize::try_from(count)
            .with_context(|| format!("converting `{table}` representation count to usize"))?;
        match normalize_representation_kind(kind.trim()) {
            Some("code") => counts.code += count,
            Some("summary") => counts.summary += count,
            _ => {}
        }
    }
    Ok(counts)
}

fn load_current_joined_representation_kind_counts_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<RepresentationKindCounts> {
    let sql = "SELECT e.representation_kind, COUNT(*) \
               FROM artefacts_current a \
               JOIN symbol_embeddings e \
                 ON e.repo_id = a.repo_id \
                AND e.artefact_id = a.artefact_id \
               WHERE a.repo_id = ?1 \
               GROUP BY e.representation_kind";
    let mut counts = RepresentationKindCounts::default();
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) if is_missing_table_or_column_error(&err) => return Ok(counts),
        Err(err) => return Err(err).context("preparing current joined representation query"),
    };
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .context("querying current joined representation kinds")?;
    for row in rows {
        let (kind, count) = row.context("reading current joined representation row")?;
        let count = usize::try_from(count)
            .context("converting current joined representation count to usize")?;
        match normalize_representation_kind(kind.trim()) {
            Some("code") => counts.code += count,
            Some("summary") => counts.summary += count,
            _ => {}
        }
    }
    Ok(counts)
}

fn load_semantic_clone_table_snapshot(world: &QatWorld) -> Result<SemanticCloneTableSnapshot> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    Ok(SemanticCloneTableSnapshot {
        historical: SemanticCloneHistoricalTableSnapshot {
            artefacts_historical: count_rows_for_repo(&conn, "artefacts_historical", &repo_id)?,
            symbol_features: count_rows_for_repo(&conn, "symbol_features", &repo_id)?,
            symbol_semantics: count_rows_for_repo(&conn, "symbol_semantics", &repo_id)?,
            symbol_embeddings: count_rows_for_repo(&conn, "symbol_embeddings", &repo_id)?,
            commit_ingest_ledger: count_rows_for_repo(&conn, "commit_ingest_ledger", &repo_id)?,
        },
        current: SemanticCloneCurrentTableSnapshot {
            artefacts_current: count_rows_for_repo(&conn, "artefacts_current", &repo_id)?,
            symbol_features_current: count_rows_for_repo(
                &conn,
                "symbol_features_current",
                &repo_id,
            )?,
            symbol_semantics_current: count_rows_for_repo(
                &conn,
                "symbol_semantics_current",
                &repo_id,
            )?,
            symbol_embeddings_current: count_rows_for_repo(
                &conn,
                "symbol_embeddings_current",
                &repo_id,
            )?,
            symbol_clone_edges: count_rows_for_repo(&conn, "symbol_clone_edges", &repo_id)?,
        },
        historical_representation_counts: load_representation_kind_counts_for_repo(
            &conn,
            "symbol_embeddings",
            &repo_id,
        )?,
        current_representation_counts: load_representation_kind_counts_for_repo(
            &conn,
            "symbol_embeddings_current",
            &repo_id,
        )?,
        current_joined_representation_counts: load_current_joined_representation_kind_counts_for_repo(
            &conn,
            &repo_id,
        )?,
    })
}

fn store_semantic_clone_table_snapshot(world: &mut QatWorld, snapshot: SemanticCloneTableSnapshot) {
    world.semantic_clone_table_snapshot = Some(snapshot);
}

fn describe_semantic_clone_table_snapshot(snapshot: &SemanticCloneTableSnapshot) -> String {
    format!(
        "historical(artefacts={}, features={}, semantics={}, embeddings={}, ledger={}); current(artefacts={}, features={}, semantics={}, embeddings={}, clone_edges={}); historical_kinds(code={}, summary={}); current_kinds(code={}, summary={}); current_joined_kinds(code={}, summary={})",
        snapshot.historical.artefacts_historical,
        snapshot.historical.symbol_features,
        snapshot.historical.symbol_semantics,
        snapshot.historical.symbol_embeddings,
        snapshot.historical.commit_ingest_ledger,
        snapshot.current.artefacts_current,
        snapshot.current.symbol_features_current,
        snapshot.current.symbol_semantics_current,
        snapshot.current.symbol_embeddings_current,
        snapshot.current.symbol_clone_edges,
        snapshot.historical_representation_counts.code,
        snapshot.historical_representation_counts.summary,
        snapshot.current_representation_counts.code,
        snapshot.current_representation_counts.summary,
        snapshot.current_joined_representation_counts.code,
        snapshot.current_joined_representation_counts.summary,
    )
}

fn load_semantic_clone_store_snapshot(world: &QatWorld) -> Result<SemanticCloneStoreSnapshot> {
    let path = relational_db_path_for_world(world)?;
    let snapshot = load_semantic_clone_table_snapshot(world)?;
    let embeddings = snapshot
        .historical
        .symbol_embeddings
        .max(snapshot.current.symbol_embeddings_current);
    Ok(SemanticCloneStoreSnapshot {
        path,
        evidence: SemanticCloneStoreEvidence {
            current_artefacts: snapshot.current.artefacts_current,
            embeddings,
            clone_edges: snapshot.current.symbol_clone_edges,
        },
    })
}

fn describe_semantic_clone_store_snapshot(snapshot: &SemanticCloneStoreSnapshot) -> String {
    format!(
        "db={}, current_artefacts={}, embeddings={}, clone_edges={}",
        snapshot.path.display(),
        snapshot.evidence.current_artefacts,
        snapshot.evidence.embeddings,
        snapshot.evidence.clone_edges
    )
}

fn semantic_clone_eventual_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(SEMANTIC_CLONES_EVENTUAL_TIMEOUT_ENV)
            .ok()
            .as_deref(),
        DEFAULT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS,
    )
}

fn semantic_clone_eventual_poll_interval() -> StdDuration {
    StdDuration::from_millis(SEMANTIC_CLONES_EVENTUAL_POLL_INTERVAL_MILLIS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloneQueryWaitCondition {
    AnyResponse,
    NonEmptyResults,
}

fn clone_query_meets_wait_condition(
    rows: &[serde_json::Value],
    condition: &CloneQueryWaitCondition,
) -> bool {
    match condition {
        CloneQueryWaitCondition::AnyResponse => true,
        CloneQueryWaitCondition::NonEmptyResults => !rows.is_empty(),
    }
}

fn wait_for_semantic_clone_condition<T, Observe, Ready, Describe>(
    timeout: StdDuration,
    poll_interval: StdDuration,
    expected: &str,
    mut observe: Observe,
    is_ready: Ready,
    describe: Describe,
) -> Result<T>
where
    Observe: FnMut() -> Result<T>,
    Ready: Fn(&T) -> bool,
    Describe: Fn(&T) -> String,
{
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_observation: String;

    loop {
        attempts += 1;
        match observe() {
            Ok(value) => {
                let summary = describe(&value);
                if is_ready(&value) {
                    return Ok(value);
                }
                last_observation = format!("value: {summary}");
            }
            Err(err) => {
                last_observation = format!("error: {err:#}");
            }
        }

        if started.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for semantic clone {expected}; attempts={attempts}; last observation={}",
                timeout.as_secs(),
                last_observation
            );
        }

        std::thread::sleep(poll_interval);
    }
}

fn wait_for_semantic_clone_store_snapshot(world: &QatWorld) -> Result<SemanticCloneStoreSnapshot> {
    wait_for_semantic_clone_condition(
        semantic_clone_eventual_timeout(),
        semantic_clone_eventual_poll_interval(),
        "persisted semantic clone evidence",
        || load_semantic_clone_store_snapshot(world),
        |snapshot| semantic_clone_store_evidence_proves_rebuild(None, snapshot.evidence),
        describe_semantic_clone_store_snapshot,
    )
}

fn write_semantic_clone_fixture_files(repo_dir: &Path, write_project_files: bool) -> Result<()> {
    let src = repo_dir.join("src");
    let handlers = src.join("handlers");
    let render = src.join("render");
    fs::create_dir_all(&handlers).with_context(|| format!("creating {}", handlers.display()))?;
    fs::create_dir_all(&render).with_context(|| format!("creating {}", render.display()))?;

    if write_project_files {
        fs::write(
            repo_dir.join("package.json"),
            "{\n  \"name\": \"qat-semantic-clones-fixture\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\"\n}\n",
        )
        .context("writing package.json")?;
        fs::write(
            repo_dir.join("tsconfig.json"),
            "{\n  \"compilerOptions\": {\n    \"target\": \"ES2020\",\n    \"module\": \"ESNext\",\n    \"moduleResolution\": \"bundler\",\n    \"strict\": true,\n    \"outDir\": \"dist\"\n  },\n  \"include\": [\"src\"]\n}\n",
        )
        .context("writing tsconfig.json")?;
    }

    fs::write(
        handlers.join("common-snapshot-utils.ts"),
        r#"export function normalizeSnapshotKey(componentId: string): string {
  return componentId.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

export function normalizeWorkspaceKey(workspaceId: string): string {
  return workspaceId.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

export function loadSnapshotRecord(componentId: string): string {
  return `snapshot:${normalizeSnapshotKey(componentId)}`;
}

export function persistSnapshotRecord(snapshot: string): string {
  return `persisted:${snapshot}`;
}

export function buildSnapshotChecksum(snapshot: string): string {
  return `checksum:${snapshot.length}`;
}

export function appendSnapshotAuditLine(snapshot: string, workspaceId: string): string {
  return `${snapshot}:audit:${normalizeWorkspaceKey(workspaceId)}`;
}

export function resolveSnapshotDisplayName(componentId: string): string {
  return `display:${normalizeSnapshotKey(componentId)}`;
}

export function resolveSnapshotPrefix(workspaceId: string): string {
  return `workspace:${normalizeWorkspaceKey(workspaceId)}`;
}

export function formatSnapshotEnvelope(snapshot: string, workspaceId: string): string {
  return `${resolveSnapshotPrefix(workspaceId)}|${snapshot}`;
}

export function splitSnapshotEnvelope(envelope: string): string[] {
  return envelope.split("|");
}

export function dedupeSnapshotKeys(values: string[]): string[] {
  return Array.from(new Set(values));
}

export function sortSnapshotKeys(values: string[]): string[] {
  return [...values].sort();
}

export function createSnapshotTimelineEntry(snapshot: string, workspaceId: string): string {
  return `${appendSnapshotAuditLine(snapshot, workspaceId)}:${buildSnapshotChecksum(snapshot)}`;
}

export function summarizeSnapshotTimeline(snapshot: string, workspaceId: string): string {
  return createSnapshotTimelineEntry(snapshot, workspaceId);
}

export function buildComponentSnapshotChangeSet(snapshot: string): string {
  return `${snapshot}:changes:${buildSnapshotChecksum(snapshot)}`;
}

export function formatBelongsToRelationship(snapshot: string): string {
  return `${snapshot}:belongs-to`;
}

export function formatInstanceInRelationship(snapshot: string): string {
  return `${snapshot}:instance-in`;
}

export function createRenderHeader(orderId: string, locale: string): string {
  return `header:${orderId}:${locale}`;
}

export function createRenderBody(orderId: string, locale: string): string {
  return `body:${orderId}:${locale}`;
}

export function createRenderFooter(orderId: string, locale: string): string {
  return `footer:${orderId}:${locale}`;
}
"#,
    )
    .context("writing src/handlers/common-snapshot-utils.ts")?;

    fs::write(
        handlers.join("create-component-snapshots.handler.ts"),
        r#"import {
  buildComponentSnapshotChangeSet,
  formatBelongsToRelationship,
  loadSnapshotRecord,
  persistSnapshotRecord,
  summarizeSnapshotTimeline,
} from "./common-snapshot-utils";

export class CreateComponentSnapshotsCommandHandler {
  async execute(componentId: string, workspaceId: string): Promise<string> {
    const snapshot = await this.loadSnapshot(componentId, workspaceId);
    const relationshipChanges = buildComponentSnapshotChangeSet(snapshot);
    return this.persistSnapshot(relationshipChanges);
  }

  async updateSnapshotRelationshipsForBelongsToSnapshotRelationship(
    componentId: string,
    workspaceId: string,
  ): Promise<string> {
    const snapshot = await this.loadSnapshot(componentId, workspaceId);
    return formatBelongsToRelationship(snapshot);
  }

  private async loadSnapshot(componentId: string, workspaceId: string): Promise<string> {
    const snapshot = loadSnapshotRecord(componentId);
    return summarizeSnapshotTimeline(snapshot, workspaceId);
  }

  private persistSnapshot(snapshot: string): Promise<string> {
    return Promise.resolve(persistSnapshotRecord(snapshot));
  }
}
"#,
    )
    .context("writing src/handlers/create-component-snapshots.handler.ts")?;

    fs::write(
        handlers.join("sync-component-snapshots.handler.ts"),
        r#"import {
  buildComponentSnapshotChangeSet,
  formatInstanceInRelationship,
  loadSnapshotRecord,
  persistSnapshotRecord,
  summarizeSnapshotTimeline,
} from "./common-snapshot-utils";

export class SyncComponentSnapshotsCommandHandler {
  async execute(componentId: string, workspaceId: string): Promise<string> {
    const snapshot = await this.loadSnapshot(componentId, workspaceId);
    const relationshipChanges = buildComponentSnapshotChangeSet(snapshot);
    return this.persistSnapshot(relationshipChanges);
  }

  async updateSnapshotRelationshipsForInstanceInSnapshotRelationship(
    componentId: string,
    workspaceId: string,
  ): Promise<string> {
    const snapshot = await this.loadSnapshot(componentId, workspaceId);
    return formatInstanceInRelationship(snapshot);
  }

  private async loadSnapshot(componentId: string, workspaceId: string): Promise<string> {
    const snapshot = loadSnapshotRecord(componentId);
    return summarizeSnapshotTimeline(snapshot, workspaceId);
  }

  private persistSnapshot(snapshot: string): Promise<string> {
    return Promise.resolve(persistSnapshotRecord(snapshot));
  }
}
"#,
    )
    .context("writing src/handlers/sync-component-snapshots.handler.ts")?;

    fs::write(
        render.join("render-invoice.ts"),
        r#"import {
  createRenderBody,
  createRenderFooter,
  createRenderHeader,
} from "../handlers/common-snapshot-utils";

export function renderInvoice(orderId: string, locale: string): string {
  return [createRenderHeader(orderId, locale), createRenderBody(orderId, locale)].join("\n");
}

export function renderInvoiceSummary(orderId: string, locale: string): string {
  return `${orderId}:${locale}:summary`;
}

export function renderInvoiceMetadata(orderId: string, locale: string): string {
  return `${orderId}:${locale}:metadata`;
}

export function renderInvoiceFooter(orderId: string, locale: string): string {
  return createRenderFooter(orderId, locale);
}

export function renderInvoiceAuditTrail(orderId: string, locale: string): string {
  return `${renderInvoiceSummary(orderId, locale)}:${renderInvoiceFooter(orderId, locale)}`;
}
"#,
    )
    .context("writing src/render/render-invoice.ts")?;

    fs::write(
        render.join("render-invoice-document.ts"),
        r#"import {
  createRenderBody,
  createRenderFooter,
  createRenderHeader,
} from "../handlers/common-snapshot-utils";

export function renderInvoiceDocument(orderId: string, locale: string): string {
  return [createRenderHeader(orderId, locale), createRenderBody(orderId, locale)].join("\n");
}

export function renderInvoiceDocumentSummary(orderId: string, locale: string): string {
  return `${orderId}:${locale}:summary`;
}

export function renderInvoiceDocumentMetadata(orderId: string, locale: string): string {
  return `${orderId}:${locale}:metadata`;
}

export function renderInvoiceDocumentFooter(orderId: string, locale: string): string {
  return createRenderFooter(orderId, locale);
}

export function renderInvoiceDocumentAuditTrail(orderId: string, locale: string): string {
  return `${renderInvoiceDocumentSummary(orderId, locale)}:${renderInvoiceDocumentFooter(orderId, locale)}`;
}
"#,
    )
    .context("writing src/render/render-invoice-document.ts")?;

    Ok(())
}

pub fn create_ts_project_with_similar_impls(repo_dir: &Path) -> Result<()> {
    write_semantic_clone_fixture_files(repo_dir, true)
}

pub fn add_semantic_clone_fixtures(repo_dir: &Path) -> Result<()> {
    write_semantic_clone_fixture_files(repo_dir, false)
}

pub fn run_devql_semantic_clones_rebuild(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql tasks enqueue --kind ingest --status",
        build_bitloops_command(
            world,
            &["devql", "tasks", "enqueue", "--kind", "ingest", "--status"],
        )?,
    )?;
    ensure_success(&output, "bitloops devql tasks enqueue --kind ingest --status")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let semantic_rows = extract_ingest_metric(&stdout, "semantic_feature_rows_upserted=")
        .ok_or_else(|| {
            anyhow!(
                "bitloops devql tasks enqueue --kind ingest --status completed but did not report semantic_feature_rows_upserted=... in stdout; semantic clones rebuild requires ingest metrics to verify clone setup"
            )
        })?;
    let clone_edges = extract_ingest_metric(&stdout, "symbol_clone_edges_upserted=");
    run_devql_sync_for_repo(world, repo_name)?;
    wait_for_semantic_clone_enrichments_to_drain(world, repo_name)?;
    let store_snapshot = wait_for_semantic_clone_store_snapshot(world).with_context(|| {
        format!(
            "bitloops devql tasks enqueue --kind ingest --status reported semantic_feature_rows_upserted={semantic_rows}, symbol_clone_edges_upserted={clone_edges:?}"
        )
    })?;
    let table_snapshot = load_semantic_clone_table_snapshot(world)?;
    store_semantic_clone_table_snapshot(world, table_snapshot);
    let store_evidence = store_snapshot.evidence;
    if clone_edges.is_none() {
        append_world_log(
            world,
            &format!(
                "Semantic clone ingest stdout omitted symbol_clone_edges_upserted; waited for persisted evidence in {} with current_artefacts={}, embeddings={}, clone_edges={}.\n",
                store_snapshot.path.display(),
                store_evidence.current_artefacts,
                store_evidence.embeddings,
                store_evidence.clone_edges
            ),
        )?;
    } else if clone_edges == Some(0)
        && semantic_clone_store_evidence_proves_rebuild(clone_edges, store_evidence)
    {
        append_world_log(
            world,
            &format!(
                "Semantic clone ingest reported zero clone edges; waited for persisted evidence in {} with current_artefacts={}, embeddings={}, clone_edges={}.\n",
                store_snapshot.path.display(),
                store_evidence.current_artefacts,
                store_evidence.embeddings,
                store_evidence.clone_edges
            ),
        )?;
    }
    ensure!(
        semantic_clone_store_evidence_proves_rebuild(clone_edges, store_evidence),
        "bitloops devql semantic clones rebuild succeeded but did not leave persisted semantic clone evidence in {} (semantic_feature_rows_upserted={semantic_rows}, symbol_clone_edges_upserted={clone_edges:?}, current_artefacts={}, symbol_embeddings={}, symbol_clone_edges={}). Re-run `bitloops devql tasks enqueue --kind sync --status` and `bitloops devql tasks enqueue --kind ingest --status` and inspect the semantic provider output.",
        store_snapshot.path.display(),
        store_evidence.current_artefacts,
        store_evidence.embeddings,
        store_evidence.clone_edges
    );
    Ok(())
}

fn parse_status_u64_line(line: &str, prefix: &str) -> Option<u64> {
    line.strip_prefix(prefix)?.trim().parse::<u64>().ok()
}

fn parse_status_text_line(line: &str, prefix: &str) -> Option<String> {
    Some(line.strip_prefix(prefix)?.trim().to_string())
}

fn parse_status_bool_line(line: &str, prefix: &str) -> Option<bool> {
    match line.strip_prefix(prefix)?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn parse_enrichment_status_snapshot(stdout: &str) -> Result<EnrichmentStatusSnapshot> {
    let mut snapshot = EnrichmentStatusSnapshot::default();
    for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = parse_status_text_line(line, "Enrichment mode: ") {
            snapshot.mode = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment pending jobs: ")
        {
            snapshot.pending_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment pending semantic jobs: ")
        {
            snapshot.pending_semantic_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment pending embedding jobs: ")
        {
            snapshot.pending_embedding_jobs = value;
        } else if let Some(value) = parse_status_u64_line(
            line,
            "Enrichment pending clone-edge rebuild jobs: ",
        ) {
            snapshot.pending_clone_edges_rebuild_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment running jobs: ")
        {
            snapshot.running_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment running semantic jobs: ")
        {
            snapshot.running_semantic_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment running embedding jobs: ")
        {
            snapshot.running_embedding_jobs = value;
        } else if let Some(value) = parse_status_u64_line(
            line,
            "Enrichment running clone-edge rebuild jobs: ",
        ) {
            snapshot.running_clone_edges_rebuild_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment failed jobs: ")
        {
            snapshot.failed_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment failed semantic jobs: ")
        {
            snapshot.failed_semantic_jobs = value;
        } else if let Some(value) =
            parse_status_u64_line(line, "Enrichment failed embedding jobs: ")
        {
            snapshot.failed_embedding_jobs = value;
        } else if let Some(value) = parse_status_u64_line(
            line,
            "Enrichment failed clone-edge rebuild jobs: ",
        ) {
            snapshot.failed_clone_edges_rebuild_jobs = value;
        } else if let Some(value) = parse_status_u64_line(
            line,
            "Enrichment retried failed jobs: ",
        ) {
            snapshot.retried_failed_jobs = value;
        } else if let Some(value) =
            parse_status_text_line(line, "Enrichment last action: ")
        {
            snapshot.last_action = Some(value);
        } else if let Some(value) =
            parse_status_text_line(line, "Enrichment pause reason: ")
        {
            snapshot.paused_reason = Some(value);
        } else if let Some(value) =
            parse_status_bool_line(line, "Enrichment persisted: ")
        {
            snapshot.persisted = value;
        }
    }

    ensure!(
        !snapshot.mode.is_empty(),
        "could not parse `Enrichment mode:` from daemon enrichments status output:\n{stdout}"
    );
    Ok(snapshot)
}

fn describe_enrichment_status(snapshot: &EnrichmentStatusSnapshot) -> String {
    format!(
        "mode={}, pending={} (semantic={}, embedding={}, clone_edges={}), running={} (semantic={}, embedding={}, clone_edges={}), failed={} (semantic={}, embedding={}, clone_edges={}), retried_failed={}, persisted={}, last_action={:?}, paused_reason={:?}",
        snapshot.mode,
        snapshot.pending_jobs,
        snapshot.pending_semantic_jobs,
        snapshot.pending_embedding_jobs,
        snapshot.pending_clone_edges_rebuild_jobs,
        snapshot.running_jobs,
        snapshot.running_semantic_jobs,
        snapshot.running_embedding_jobs,
        snapshot.running_clone_edges_rebuild_jobs,
        snapshot.failed_jobs,
        snapshot.failed_semantic_jobs,
        snapshot.failed_embedding_jobs,
        snapshot.failed_clone_edges_rebuild_jobs,
        snapshot.retried_failed_jobs,
        snapshot.persisted,
        snapshot.last_action,
        snapshot.paused_reason,
    )
}

fn enrichments_have_no_failures(snapshot: &EnrichmentStatusSnapshot) -> bool {
    snapshot.failed_jobs == 0
        && snapshot.failed_semantic_jobs == 0
        && snapshot.failed_embedding_jobs == 0
        && snapshot.failed_clone_edges_rebuild_jobs == 0
}

fn enrichments_drained(snapshot: &EnrichmentStatusSnapshot) -> bool {
    snapshot.pending_jobs == 0 && snapshot.running_jobs == 0
}

pub fn run_daemon_enrichments_status(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<EnrichmentStatusSnapshot> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops daemon enrichments status",
        build_bitloops_command(world, &["daemon", "enrichments", "status"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, "bitloops daemon enrichments status")?;
    let snapshot = parse_enrichment_status_snapshot(&stdout)?;
    world.last_enrichment_status_snapshot = Some(snapshot.clone());
    Ok(snapshot)
}

pub fn wait_for_semantic_clone_enrichments_to_drain(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let snapshot = wait_for_semantic_clone_condition(
        semantic_clone_eventual_timeout(),
        semantic_clone_eventual_poll_interval(),
        "semantic-clone enrichments to drain",
        || run_daemon_enrichments_status(world, repo_name),
        |snapshot| enrichments_drained(snapshot) && enrichments_have_no_failures(snapshot),
        describe_enrichment_status,
    )?;
    world.last_enrichment_status_snapshot = Some(snapshot.clone());
    ensure!(
        enrichments_have_no_failures(&snapshot),
        "expected daemon enrichments to drain without failures, got {}",
        describe_enrichment_status(&snapshot)
    );
    Ok(())
}

pub fn observe_semantic_clone_enrichment_progress(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let timeout = semantic_clone_eventual_timeout();
    let poll_interval = semantic_clone_eventual_poll_interval();
    let started = Instant::now();
    let mut observation = SemanticCloneProgressObservation::default();
    let mut previous_status: Option<EnrichmentStatusSnapshot> = None;
    let mut last_status: EnrichmentStatusSnapshot;

    loop {
        let status = run_daemon_enrichments_status(world, repo_name)?;
        ensure!(
            enrichments_have_no_failures(&status),
            "expected daemon enrichments to stay failure-free during semantic-clone progress observation, got {}",
            describe_enrichment_status(&status)
        );
        observation.status_samples += 1;
        observation.max_pending_embedding_jobs = observation
            .max_pending_embedding_jobs
            .max(status.pending_embedding_jobs);
        observation.max_pending_clone_edges_rebuild_jobs = observation
            .max_pending_clone_edges_rebuild_jobs
            .max(status.pending_clone_edges_rebuild_jobs);
        observation.embedding_pending_decreased |=
            observation.max_pending_embedding_jobs > status.pending_embedding_jobs;
        if let Some(previous) = &previous_status {
            let pending_drop = previous.pending_jobs.saturating_sub(status.pending_jobs);
            let embedding_drop = previous
                .pending_embedding_jobs
                .saturating_sub(status.pending_embedding_jobs);
            if status.running_jobs >= 2 || pending_drop >= 2 || embedding_drop >= 2 {
                observation.parallel_progress_observed = true;
            }
        }
        if status.pending_embedding_jobs > 0 || status.running_embedding_jobs > 0 {
            observation.embedding_activity_observed = true;
        }
        if status.pending_clone_edges_rebuild_jobs > 0
            || status.running_clone_edges_rebuild_jobs > 0
        {
            observation.clone_edges_rebuild_observed = true;
        }

        if let Ok(snapshot) = load_semantic_clone_table_snapshot(world) {
            if snapshot.historical_representation_counts.code > 0
                && observation.first_code_embedding_count == 0
            {
                observation.first_code_embedding_count =
                    snapshot.historical_representation_counts.code;
                observation.code_embeddings_appeared_before_drain = !enrichments_drained(&status);
            }
            if snapshot.historical_representation_counts.summary > 0
                && observation.first_summary_embedding_count == 0
            {
                observation.first_summary_embedding_count =
                    snapshot.historical_representation_counts.summary;
                observation.summary_embeddings_appeared_before_drain =
                    !enrichments_drained(&status);
            }
            store_semantic_clone_table_snapshot(world, snapshot);
        }

        previous_status = Some(status.clone());
        last_status = status.clone();

        if started.elapsed() >= timeout || enrichments_drained(&status) {
            break;
        }

        std::thread::sleep(poll_interval);
    }

    world.semantic_clone_progress_observation = Some(observation.clone());
    world.last_enrichment_status_snapshot = Some(last_status);
    if let Ok(snapshot) = load_semantic_clone_table_snapshot(world) {
        store_semantic_clone_table_snapshot(world, snapshot);
    }

    ensure!(
        observation.status_samples >= 2,
        "expected at least 2 daemon enrichments status samples, observed {}",
        observation.status_samples
    );
    ensure!(
        observation.embedding_activity_observed,
        "expected embedding queue activity during semantic-clone observation, got {:?}",
        world.last_enrichment_status_snapshot
    );
    ensure!(
        observation.clone_edges_rebuild_observed,
        "expected clone-edge rebuild queue activity during semantic-clone observation, got {:?}",
        world.last_enrichment_status_snapshot
    );
    ensure!(
        observation.parallel_progress_observed,
        "expected to observe guide-aligned parallel worker progress while embeddings were draining, got {:?}",
        world.semantic_clone_progress_observation
    );
    ensure!(
        observation.embedding_pending_decreased,
        "expected pending embedding jobs to decrease over the observation window, got {:?}",
        world.semantic_clone_progress_observation
    );
    ensure!(
        observation.first_code_embedding_count > 0,
        "expected historical `code` embeddings to appear during observation, got {:?}",
        world.semantic_clone_table_snapshot
    );
    ensure!(
        observation.code_embeddings_appeared_before_drain,
        "expected current `code` embeddings to appear before the enrichment queue fully drained, got {:?}",
        world.semantic_clone_progress_observation
    );
    ensure!(
        observation.first_summary_embedding_count > 0,
        "expected historical `summary` embeddings to appear during observation, got {:?}",
        world.semantic_clone_table_snapshot
    );
    ensure!(
        observation.summary_embeddings_appeared_before_drain,
        "expected current `summary` embeddings to appear before the enrichment queue fully drained, got {:?}",
        world.semantic_clone_progress_observation
    );
    Ok(())
}

pub fn assert_semantic_clone_historical_tables_populated(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let snapshot = load_semantic_clone_table_snapshot(world)?;
    let message = describe_semantic_clone_table_snapshot(&snapshot);
    store_semantic_clone_table_snapshot(world, snapshot.clone());
    ensure!(
        snapshot.historical.artefacts_historical > 0,
        "expected historical artefacts after ingest, got {message}"
    );
    ensure!(
        snapshot.historical.symbol_features > 0,
        "expected historical symbol features after ingest, got {message}"
    );
    ensure!(
        snapshot.historical.symbol_semantics > 0,
        "expected historical symbol semantics after ingest, got {message}"
    );
    ensure!(
        snapshot.historical.commit_ingest_ledger > 0,
        "expected commit_ingest_ledger rows after ingest, got {message}"
    );
    Ok(())
}

pub fn assert_semantic_clone_current_tables_populated(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let snapshot = load_semantic_clone_table_snapshot(world)?;
    let message = describe_semantic_clone_table_snapshot(&snapshot);
    store_semantic_clone_table_snapshot(world, snapshot.clone());
    ensure!(
        snapshot.current.artefacts_current > 0,
        "expected current artefacts after sync, got {message}"
    );
    ensure!(
        snapshot.current.symbol_features_current > 0,
        "expected current symbol features after sync, got {message}"
    );
    ensure!(
        snapshot.current.symbol_semantics_current > 0,
        "expected current symbol semantics after sync, got {message}"
    );
    ensure!(
        snapshot.current.symbol_clone_edges > 0,
        "expected current symbol clone edges after sync and enrichments, got {message}"
    );
    ensure!(
        snapshot.current.symbol_embeddings_current == 0,
        "expected current inline symbol embeddings to remain empty after sync, got {message}"
    );
    Ok(())
}

pub fn assert_semantic_clone_representation_channels_populated(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let snapshot = load_semantic_clone_table_snapshot(world)?;
    let message = describe_semantic_clone_table_snapshot(&snapshot);
    store_semantic_clone_table_snapshot(world, snapshot.clone());
    ensure!(
        snapshot.historical_representation_counts.code > 0,
        "expected historical `code` embeddings, got {message}"
    );
    ensure!(
        snapshot.historical_representation_counts.summary > 0,
        "expected historical `summary` embeddings, got {message}"
    );
    ensure!(
        snapshot.current_joined_representation_counts.code > 0,
        "expected current artefacts to resolve `code` embeddings from the historical store, got {message}"
    );
    ensure!(
        snapshot.current_joined_representation_counts.summary > 0,
        "expected current artefacts to resolve `summary` embeddings from the historical store, got {message}"
    );
    ensure!(
        snapshot.current.symbol_embeddings_current == 0,
        "expected current inline embedding rows to remain empty after sync, got {message}"
    );
    ensure!(
        snapshot.current.symbol_clone_edges > 0,
        "expected current clone edges to remain populated, got {message}"
    );
    Ok(())
}

fn extract_clone_nodes(value: &serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(rows) => {
            if rows.iter().any(|row| {
                row.get("relationKind").is_some()
                    || row.get("sourceArtefactId").is_some()
                    || row.get("from").is_some()
            }) {
                return rows.clone();
            }
            rows.iter()
                .flat_map(|artefact| {
                    artefact
                        .get("clones")
                        .and_then(|clones| clones.get("edges"))
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|edge| edge.get("node").cloned())
                })
                .collect()
        }
        serde_json::Value::Object(object) => {
            if let Some(repo) = object.get("repo") {
                return extract_clone_nodes(repo);
            }
            if let Some(data) = object.get("data") {
                return extract_clone_nodes(data);
            }
            if let Some(artefacts) = object.get("artefacts") {
                return artefacts
                    .get("edges")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|edge| {
                        edge.get("node")
                            .and_then(|node| node.get("clones"))
                            .and_then(|clones| clones.get("edges"))
                            .and_then(serde_json::Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|clone_edge| clone_edge.get("node").cloned())
                    })
                    .collect();
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn clone_target_symbol_fqn(row: &serde_json::Value) -> Option<&str> {
    row.get("targetArtefact")
        .and_then(|artefact| artefact.get("symbolFqn"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| row.get("to").and_then(serde_json::Value::as_str))
        .or_else(|| row.get("target_symbol_fqn").and_then(serde_json::Value::as_str))
}

fn run_devql_clones_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
) -> Result<Vec<serde_json::Value>> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let mut clone_args = Vec::new();
    if let Some(min_score) = min_score {
        clone_args.push(format!("min_score:{min_score}"));
    }
    if raw {
        clone_args.push("raw:true".to_string());
    }
    let clones_stage = if clone_args.is_empty() {
        "clones()".to_string()
    } else {
        format!("clones({})", clone_args.join(","))
    };
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->{}->limit(50)"#,
        escape_devql_string(&symbol_fqn),
        clones_stage
    );
    let value = run_devql_query(world, &query)?;
    let clone_rows = extract_clone_nodes(&value);
    world.last_command_stdout =
        Some(serde_json::to_string(&clone_rows).context("serializing flattened clone rows")?);
    world.last_query_result_count = Some(clone_rows.len());
    Ok(clone_rows)
}

fn describe_clone_rows(rows: &[serde_json::Value]) -> String {
    serde_json::to_string(rows).unwrap_or_else(|_| "<failed to serialize clone rows>".to_string())
}

fn max_clone_score(rows: &[serde_json::Value]) -> f64 {
    rows.iter()
        .filter_map(|row| row.get("score").and_then(serde_json::Value::as_f64))
        .fold(0.0_f64, f64::max)
}

fn clone_rows_have_explanation(rows: &[serde_json::Value]) -> bool {
    rows.iter().any(|row| {
        row.get("explanation_json")
            .or_else(|| row.get("metadata"))
            .and_then(|metadata| metadata.get("explanation").or(Some(metadata)))
            .is_some_and(|metadata| match metadata {
                serde_json::Value::Null => false,
                serde_json::Value::Object(map) => !map.is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::String(text) => !text.trim().is_empty(),
                _ => true,
            })
    })
}

fn run_devql_clones_query_eventually<Condition>(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
    expected: &str,
    condition: Condition,
) -> Result<Vec<serde_json::Value>>
where
    Condition: Fn(&[serde_json::Value]) -> bool,
{
    wait_for_semantic_clone_condition(
        semantic_clone_eventual_timeout(),
        semantic_clone_eventual_poll_interval(),
        expected,
        || run_devql_clones_query(world, repo_name, symbol_alias, min_score, raw),
        |rows| condition(rows.as_slice()),
        |rows| describe_clone_rows(rows.as_slice()),
    )
}

fn run_devql_clones_query_eventually_with_wait_condition(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: Option<f64>,
    raw: bool,
    expected: &str,
    wait_condition: CloneQueryWaitCondition,
) -> Result<Vec<serde_json::Value>> {
    run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        min_score,
        raw,
        expected,
        |rows| clone_query_meets_wait_condition(rows, &wait_condition),
    )
}

pub fn assert_devql_clones_query(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_count: usize,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        false,
        &format!("at least {min_count} clone rows for `{symbol_alias}`"),
        |rows| rows.len() >= min_count,
    )?;
    let count = rows.len();
    ensure!(
        count >= min_count,
        "expected at least {min_count} clones for `{symbol_alias}`, got {count}"
    );
    Ok(())
}

pub fn assert_devql_clones_with_min_score(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually_with_wait_condition(
        world,
        repo_name,
        symbol_alias,
        Some(min_score),
        false,
        &format!("clone rows with min_score={min_score} for `{symbol_alias}`"),
        CloneQueryWaitCondition::NonEmptyResults,
    )?;
    let count = rows.len();
    ensure!(
        count >= 1,
        "expected at least one clone result with min_score={min_score}, got {count}"
    );
    Ok(())
}

pub fn record_devql_clones_with_min_score(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let _ = run_devql_clones_query_eventually_with_wait_condition(
        world,
        repo_name,
        symbol_alias,
        Some(min_score),
        false,
        &format!("clone rows with min_score={min_score} for `{symbol_alias}`"),
        CloneQueryWaitCondition::AnyResponse,
    )?;
    Ok(())
}

pub fn assert_last_query_fewer_or_equal(world: &QatWorld, previous_count: usize) -> Result<()> {
    let current = world
        .last_query_result_count
        .ok_or_else(|| anyhow!("no previous query result count captured"))?;
    ensure!(
        current <= previous_count,
        "expected fewer or equal results ({current} <= {previous_count})"
    );
    Ok(())
}

pub fn assert_devql_clones_have_score_and_kind(world: &QatWorld) -> Result<()> {
    let value = parse_last_command_stdout_json(world)?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected clones query to return JSON array"))?;
    ensure!(!rows.is_empty(), "expected at least one clone row");
    for row in rows {
        let has_score = row
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .is_some();
        ensure!(has_score, "clone row missing score field: {row}");
        ensure!(
            row.get("relationKind")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "clone row missing relationKind: {row}"
        );
    }
    Ok(())
}

pub fn assert_devql_clones_top_score_above(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    threshold: f64,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        false,
        &format!("a top clone score above {threshold} for `{symbol_alias}`"),
        |rows| max_clone_score(rows) > threshold,
    )?;
    let max_score = max_clone_score(&rows);
    ensure!(
        max_score > threshold,
        "expected top clone score > {threshold}, got {max_score}"
    );
    Ok(())
}

pub fn assert_devql_clones_have_explanation(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        true,
        &format!("clone explanation payload for `{symbol_alias}`"),
        clone_rows_have_explanation,
    )?;
    ensure!(!rows.is_empty(), "expected at least one clone row");
    let has_explanation = clone_rows_have_explanation(&rows);
    ensure!(
        has_explanation,
        "expected at least one clone row with explanation payload"
    );
    Ok(())
}

pub fn assert_devql_clones_rank_target_above(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    higher_rank_target: &str,
    lower_rank_target: &str,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually(
        world,
        repo_name,
        symbol_alias,
        None,
        false,
        &format!(
            "`{higher_rank_target}` to rank above `{lower_rank_target}` for `{symbol_alias}`"
        ),
        |rows| {
            let higher = rows
                .iter()
                .position(|row| clone_target_symbol_fqn(row) == Some(higher_rank_target));
            let lower = rows
                .iter()
                .position(|row| clone_target_symbol_fqn(row) == Some(lower_rank_target));
            matches!((higher, lower), (Some(h), Some(l)) if h < l)
        },
    )?;
    let higher = rows
        .iter()
        .position(|row| clone_target_symbol_fqn(row) == Some(higher_rank_target))
        .ok_or_else(|| anyhow!("missing clone target `{higher_rank_target}`"))?;
    let lower = rows
        .iter()
        .position(|row| clone_target_symbol_fqn(row) == Some(lower_rank_target))
        .ok_or_else(|| anyhow!("missing clone target `{lower_rank_target}`"))?;
    ensure!(
        higher < lower,
        "expected `{higher_rank_target}` to rank above `{lower_rank_target}`, got rows {}",
        describe_clone_rows(&rows)
    );
    Ok(())
}

pub fn assert_devql_clones_with_min_score_excludes_target(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
    excluded_target: &str,
) -> Result<()> {
    let rows = run_devql_clones_query_eventually_with_wait_condition(
        world,
        repo_name,
        symbol_alias,
        Some(min_score),
        false,
        &format!(
            "`{excluded_target}` to be excluded by min_score={min_score} for `{symbol_alias}`"
        ),
        CloneQueryWaitCondition::NonEmptyResults,
    )?;
    ensure!(
        rows.iter()
            .all(|row| clone_target_symbol_fqn(row) != Some(excluded_target)),
        "expected `{excluded_target}` to be excluded by min_score={min_score}, got rows {}",
        describe_clone_rows(&rows)
    );
    Ok(())
}

fn parse_clone_summary_group(group: &serde_json::Value) -> Result<CloneSummaryGroup> {
    let relation_kind = group
        .get("relation_kind")
        .or_else(|| group.get("relationKind"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("clone summary group missing relation kind: {group}"))?
        .to_string();
    let count = group
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("clone summary group missing count: {group}"))?;
    Ok(CloneSummaryGroup {
        relation_kind,
        count: usize::try_from(count).context("converting clone summary group count to usize")?,
    })
}

fn extract_clone_summary_from_devql_value(value: &serde_json::Value) -> Result<CloneSummary> {
    let row = match value {
        serde_json::Value::Array(rows) => rows
            .first()
            .ok_or_else(|| anyhow!("expected clone summary rows, got empty array"))?,
        serde_json::Value::Object(_) => value,
        _ => bail!("expected clone summary value to be an array or object, got {value}"),
    };
    let total_count = row
        .get("total_count")
        .or_else(|| row.get("totalCount"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("clone summary missing total_count/totalCount: {row}"))?;
    let groups = row
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("clone summary missing groups array: {row}"))?
        .iter()
        .map(parse_clone_summary_group)
        .collect::<Result<Vec<_>>>()?;
    Ok(CloneSummary {
        total_count: usize::try_from(total_count)
            .context("converting DevQL clone summary total_count to usize")?,
        groups,
    })
}

fn extract_clone_summary_from_graphql_value(value: &serde_json::Value) -> Result<CloneSummary> {
    let summary = value
        .pointer("/repo/cloneSummary")
        .or_else(|| value.pointer("/data/repo/cloneSummary"))
        .or_else(|| value.get("cloneSummary"))
        .or_else(|| value.pointer("/data/cloneSummary"))
        .ok_or_else(|| anyhow!("expected repo.cloneSummary in GraphQL result: {value}"))?;
    let total_count = summary
        .get("totalCount")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("GraphQL clone summary missing totalCount: {summary}"))?;
    let groups = summary
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("GraphQL clone summary missing groups array: {summary}"))?
        .iter()
        .map(parse_clone_summary_group)
        .collect::<Result<Vec<_>>>()?;
    Ok(CloneSummary {
        total_count: usize::try_from(total_count)
            .context("converting GraphQL clone summary totalCount to usize")?,
        groups,
    })
}

fn read_devql_clone_summary(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<CloneSummary> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let query = format!(
        r#"repo("bitloops")->artefacts(symbol_fqn:"{}")->clones(min_score:{})->summary()"#,
        escape_devql_string(&symbol_fqn),
        min_score,
    );
    let value = run_devql_query(world, &query)?;
    let summary = extract_clone_summary_from_devql_value(&value)?;
    world.last_query_result_count = Some(summary.total_count);
    Ok(summary)
}

fn read_graphql_clone_summary(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<CloneSummary> {
    ensure_bitloops_repo_name(repo_name)?;
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    let query = format!(
        r#"
query {{
  cloneSummary(
    filter: {{ symbolFqn: "{}" }}
    cloneFilter: {{ minScore: {} }}
  ) {{
    totalCount
    groups {{
      relationKind
      count
    }}
  }}
}}
"#,
        escape_devql_string(&symbol_fqn),
        min_score,
    );
    let value = run_devql_graphql_query(world, &query)?;
    let summary = extract_clone_summary_from_graphql_value(&value)?;
    world.last_query_result_count = Some(summary.total_count);
    Ok(summary)
}

fn assert_clone_summary_grouped_counts(summary: &CloneSummary, label: &str) -> Result<()> {
    ensure!(
        summary.total_count > 0,
        "expected {label} total_count > 0, got {}",
        summary.total_count
    );
    ensure!(
        !summary.groups.is_empty(),
        "expected {label} to include at least one relation group"
    );
    for group in &summary.groups {
        ensure!(
            !group.relation_kind.trim().is_empty(),
            "expected {label} groups to include relation_kind"
        );
        ensure!(
            group.count > 0,
            "expected {label} group `{}` to have count > 0",
            group.relation_kind
        );
    }
    Ok(())
}

pub fn assert_devql_clone_summary_grouped_counts(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let summary = read_devql_clone_summary(world, repo_name, symbol_alias, min_score)?;
    assert_clone_summary_grouped_counts(&summary, "DevQL clone summary")
}

pub fn assert_graphql_clone_summary_grouped_counts(
    world: &mut QatWorld,
    repo_name: &str,
    symbol_alias: &str,
    min_score: f64,
) -> Result<()> {
    let summary = read_graphql_clone_summary(world, repo_name, symbol_alias, min_score)?;
    assert_clone_summary_grouped_counts(&summary, "GraphQL clone summary")
}
