use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::config::{
    AtlassianProviderConfig, BlobStorageConfig, BlobStorageProvider, EventsBackendConfig,
    EventsProvider, ProviderConfig, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig,
};
use crate::engine::adapters::connectors::{
    ConnectorContext, ConnectorRegistry, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};
use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::RepoIdentity;
use crate::engine::devql::capabilities::knowledge::services::KnowledgeServices;
use crate::engine::devql::capabilities::knowledge::url::parse_knowledge_url;
use crate::engine::devql::capabilities::knowledge::{
    AssociateKnowledgeResult, IngestKnowledgeRequest, IngestKnowledgeResult, KnowledgeProvider,
};
use crate::engine::devql::capabilities::knowledge::{
    storage::{
        BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
    },
    types::KnowledgePayloadData,
};
use crate::engine::devql::capability_host::CapabilityConfigView;
use crate::engine::devql::capability_host::contexts::{
    CapabilityExecutionContext, CapabilityIngestContext, KnowledgeIngestContext,
};
use crate::engine::devql::capability_host::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, DocumentStoreGateway, ProvenanceBuilder,
    RelationalGateway,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};

#[derive(Debug, Clone)]
enum StubOutcome {
    Record(Box<ExternalKnowledgeRecord>),
    Error(String),
}

#[derive(Clone)]
struct StubKnowledgeAdapter {
    provider: KnowledgeProvider,
    queue: Arc<Mutex<HashMap<String, VecDeque<StubOutcome>>>>,
}

impl StubKnowledgeAdapter {
    fn new(
        provider: KnowledgeProvider,
        queue: Arc<Mutex<HashMap<String, VecDeque<StubOutcome>>>>,
    ) -> Self {
        Self { provider, queue }
    }
}

impl KnowledgeConnectorAdapter for StubKnowledgeAdapter {
    fn can_handle(
        &self,
        parsed: &crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl,
    ) -> bool {
        parsed.provider == self.provider
    }

    fn fetch<'a>(
        &'a self,
        parsed: &'a crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl,
        _ctx: &'a dyn ConnectorContext,
    ) -> crate::engine::adapters::connectors::types::BoxFuture<'a, Result<ExternalKnowledgeRecord>>
    {
        Box::pin(async move {
            if !self.can_handle(parsed) {
                bail!(
                    "stub adapter for `{}` cannot handle `{}`",
                    self.provider.as_str(),
                    parsed.provider.as_str()
                );
            }

            let mut queue = self.queue.lock().expect("stub queue lock");
            let Some(outcomes) = queue.get_mut(&parsed.canonical_external_id) else {
                bail!(
                    "no stub response queued for `{}`",
                    parsed.canonical_external_id
                );
            };
            let Some(outcome) = outcomes.pop_front() else {
                bail!(
                    "stub response queue exhausted for `{}`",
                    parsed.canonical_external_id
                );
            };

            match outcome {
                StubOutcome::Record(record) => Ok(*record),
                StubOutcome::Error(message) => Err(anyhow!(message)),
            }
        })
    }
}

struct StubConnectorRegistry {
    provider_config: ProviderConfig,
    github: StubKnowledgeAdapter,
    jira: StubKnowledgeAdapter,
    confluence: StubKnowledgeAdapter,
}

impl StubConnectorRegistry {
    fn new(
        provider_config: ProviderConfig,
        queue: Arc<Mutex<HashMap<String, VecDeque<StubOutcome>>>>,
    ) -> Self {
        Self {
            provider_config,
            github: StubKnowledgeAdapter::new(KnowledgeProvider::Github, queue.clone()),
            jira: StubKnowledgeAdapter::new(KnowledgeProvider::Jira, queue.clone()),
            confluence: StubKnowledgeAdapter::new(KnowledgeProvider::Confluence, queue),
        }
    }
}

impl ConnectorContext for StubConnectorRegistry {
    fn provider_config(&self) -> &ProviderConfig {
        &self.provider_config
    }
}

impl ConnectorRegistry for StubConnectorRegistry {
    fn knowledge_adapter_for(
        &self,
        parsed: &crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl,
    ) -> Result<&dyn KnowledgeConnectorAdapter> {
        match parsed.provider {
            KnowledgeProvider::Github => Ok(&self.github),
            KnowledgeProvider::Jira => Ok(&self.jira),
            KnowledgeProvider::Confluence => Ok(&self.confluence),
        }
    }
}

struct TestGraphGateway;
impl CanonicalGraphGateway for TestGraphGateway {}

struct TestProvenanceBuilder;
impl ProvenanceBuilder for TestProvenanceBuilder {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

pub(super) struct TestRuntimeContext {
    repo_root: PathBuf,
    repo: RepoIdentity,
    config_root: Value,
    sqlite_path: PathBuf,
    duckdb_path: PathBuf,
    relational: SqliteKnowledgeRelationalStore,
    documents: DuckdbKnowledgeDocumentStore,
    blobs: BlobKnowledgePayloadStore,
    connectors: StubConnectorRegistry,
    provenance: TestProvenanceBuilder,
    graph: TestGraphGateway,
    queue: Arc<Mutex<HashMap<String, VecDeque<StubOutcome>>>>,
}

impl CapabilityExecutionContext for TestRuntimeContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn graph(&self) -> &dyn CanonicalGraphGateway {
        &self.graph
    }
}

impl CapabilityIngestContext for TestRuntimeContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn blob_payloads(&self) -> &dyn BlobPayloadGateway {
        &self.blobs
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        &self.connectors
    }

    fn connector_context(&self) -> &dyn ConnectorContext {
        &self.connectors
    }

    fn provenance(&self) -> &dyn ProvenanceBuilder {
        &self.provenance
    }
}

impl KnowledgeIngestContext for TestRuntimeContext {
    fn relational(&self) -> &dyn RelationalGateway {
        &self.relational
    }

    fn documents(&self) -> &dyn DocumentStoreGateway {
        &self.documents
    }
}

#[derive(Debug, Clone)]
pub(super) struct RelationAssertionRecord {
    pub(super) source_knowledge_item_version_id: String,
    pub(super) target_type: String,
    pub(super) target_id: String,
    pub(super) relation_type: String,
    pub(super) association_method: String,
    pub(super) provenance_json: String,
}

pub(super) struct KnowledgeBddHarness {
    _temp: TempDir,
    pub(super) ctx: TestRuntimeContext,
    pub(super) services: KnowledgeServices,
    pub(super) ingest_history: Vec<IngestKnowledgeResult>,
    pub(super) association_history: Vec<AssociateKnowledgeResult>,
}

impl std::fmt::Debug for KnowledgeBddHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeBddHarness")
            .field("repo_root", &self.ctx.repo_root)
            .field("ingest_history_len", &self.ingest_history.len())
            .field("association_history_len", &self.association_history.len())
            .finish()
    }
}

impl KnowledgeBddHarness {
    pub(super) fn new() -> Result<Self> {
        let temp = TempDir::new()?;
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root)?;
        init_test_repo(
            repo_root.as_path(),
            "main",
            "Bitloops Test",
            "bitloops-test@example.com",
        );
        git_ok(
            repo_root.as_path(),
            &["commit", "--allow-empty", "-m", "initial"],
        );

        let repo = crate::engine::devql::resolve_repo_identity(repo_root.as_path())?;
        let provider_config = default_provider_config("https://bitloops.atlassian.net");
        let backends = test_backends(temp.path());
        let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
        let duckdb_path = backends.events.duckdb_path_or_default();

        let relational = SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(
            sqlite_path.clone(),
        )?);
        relational.initialise_schema()?;

        let documents = DuckdbKnowledgeDocumentStore::new(duckdb_path.clone());
        documents.initialise_schema()?;

        let blobs = BlobKnowledgePayloadStore::from_backend_config(repo_root.as_path(), &backends)?;

        let queue: Arc<Mutex<HashMap<String, VecDeque<StubOutcome>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let connectors = StubConnectorRegistry::new(provider_config.clone(), queue.clone());

        let config_root = json!({
            "knowledge": {
                "providers": {
                    "github": { "configured": true },
                    "jira": { "configured": true, "site_url": "https://bitloops.atlassian.net" },
                    "confluence": { "configured": true, "site_url": "https://bitloops.atlassian.net" },
                    "atlassian": { "configured": true, "site_url": "https://bitloops.atlassian.net" }
                }
            }
        });

        Ok(Self {
            _temp: temp,
            ctx: TestRuntimeContext {
                repo_root,
                repo,
                config_root,
                sqlite_path,
                duckdb_path,
                relational,
                documents,
                blobs,
                connectors,
                provenance: TestProvenanceBuilder,
                graph: TestGraphGateway,
                queue,
            },
            services: KnowledgeServices::new(),
            ingest_history: Vec::new(),
            association_history: Vec::new(),
        })
    }

    pub(super) fn head_commit(&self) -> String {
        git_ok(self.ctx.repo_root.as_path(), &["rev-parse", "HEAD"])
    }

    pub(super) fn create_empty_commit(&self, message: &str) -> String {
        git_ok(
            self.ctx.repo_root.as_path(),
            &["commit", "--allow-empty", "-m", message],
        );
        self.head_commit()
    }

    pub(super) fn seed_checkpoint(&self, checkpoint_id: &str) -> Result<()> {
        let pool = SqliteConnectionPool::connect(self.ctx.sqlite_path.clone())?;
        pool.initialise_checkpoint_schema()?;
        pool.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO checkpoints (checkpoint_id, repo_id) VALUES (?1, ?2)",
                rusqlite::params![checkpoint_id, self.ctx.repo.repo_id.as_str()],
            )?;
            Ok(())
        })
    }

    pub(super) fn seed_artefact(&self, artefact_id: &str) -> Result<()> {
        let pool = SqliteConnectionPool::connect(self.ctx.sqlite_path.clone())?;
        pool.with_connection(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS artefacts_current (
                    repo_id TEXT NOT NULL,
                    symbol_id TEXT NOT NULL,
                    artefact_id TEXT NOT NULL,
                    commit_sha TEXT NOT NULL,
                    blob_sha TEXT NOT NULL,
                    path TEXT NOT NULL,
                    language TEXT NOT NULL,
                    canonical_kind TEXT,
                    language_kind TEXT,
                    symbol_fqn TEXT,
                    parent_symbol_id TEXT,
                    parent_artefact_id TEXT,
                    start_line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    start_byte INTEGER NOT NULL,
                    end_byte INTEGER NOT NULL,
                    signature TEXT,
                    modifiers TEXT NOT NULL DEFAULT '[]',
                    docstring TEXT,
                    content_hash TEXT,
                    updated_at TEXT DEFAULT (datetime('now')),
                    PRIMARY KEY (repo_id, symbol_id)
                );
                CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
                ON artefacts_current (repo_id, artefact_id);",
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO artefacts_current \
                 (repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language, \
                  start_line, end_line, start_byte, end_byte) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    self.ctx.repo.repo_id.as_str(),
                    format!("sym-{artefact_id}"),
                    artefact_id,
                    "0000000000000000000000000000000000000000",
                    "0000000000000000000000000000000000000000",
                    "src/test.ts",
                    "typescript",
                    1,
                    5,
                    0,
                    50,
                ],
            )?;
            Ok(())
        })
    }

    pub(super) fn stub_success(
        &mut self,
        expected_provider: &str,
        url: &str,
        title: &str,
        body: &str,
        updated_at: Option<&str>,
    ) -> Result<()> {
        let parsed = parse_knowledge_url(url)?;
        assert_provider_matches(expected_provider, &parsed.provider)?;

        let record = build_record(
            &parsed,
            title,
            body,
            updated_at.unwrap_or("2026-03-20T10:00:00Z"),
        );
        self.enqueue_outcome(
            parsed.canonical_external_id,
            StubOutcome::Record(Box::new(record)),
        );
        Ok(())
    }

    pub(super) fn stub_success_sequence(
        &mut self,
        expected_provider: &str,
        url: &str,
        sequence: &[(String, String)],
    ) -> Result<()> {
        let parsed = parse_knowledge_url(url)?;
        assert_provider_matches(expected_provider, &parsed.provider)?;

        for (index, (title, body)) in sequence.iter().enumerate() {
            let updated_at = format!("2026-03-20T10:{:02}:00Z", index);
            let record = build_record(&parsed, title, body, &updated_at);
            self.enqueue_outcome(
                parsed.canonical_external_id.clone(),
                StubOutcome::Record(Box::new(record)),
            );
        }
        Ok(())
    }

    pub(super) fn stub_failure(
        &mut self,
        expected_provider: &str,
        url: &str,
        message: &str,
    ) -> Result<()> {
        let parsed = parse_knowledge_url(url)?;
        assert_provider_matches(expected_provider, &parsed.provider)?;
        self.enqueue_outcome(
            parsed.canonical_external_id,
            StubOutcome::Error(message.to_string()),
        );
        Ok(())
    }

    fn enqueue_outcome(&mut self, key: String, outcome: StubOutcome) {
        let mut queue = self.ctx.queue.lock().expect("stub queue lock");
        queue.entry(key).or_default().push_back(outcome);
    }

    pub(super) async fn add(
        &mut self,
        url: &str,
        commit: Option<&str>,
    ) -> Result<(IngestKnowledgeResult, Option<AssociateKnowledgeResult>)> {
        let (ingest, association) =
            run_add_flow(&self.services, &mut self.ctx, url, commit).await?;
        self.ingest_history.push(ingest.clone());
        if let Some(assoc) = association.as_ref() {
            self.association_history.push(assoc.clone());
        }
        Ok((ingest, association))
    }

    pub(super) async fn associate(
        &mut self,
        source_ref: &str,
        target_ref: &str,
    ) -> Result<AssociateKnowledgeResult> {
        let association =
            run_associate_flow(&self.services, &mut self.ctx, source_ref, target_ref).await?;
        self.association_history.push(association.clone());
        Ok(association)
    }

    pub(super) fn sqlite_row_count(&self, table: &str) -> Result<i64> {
        if !self.ctx.sqlite_path.exists() {
            return Ok(0);
        }
        let conn = rusqlite::Connection::open(&self.ctx.sqlite_path)
            .with_context(|| format!("opening sqlite db at {}", self.ctx.sqlite_path.display()))?;
        let exists = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )?;
        if exists == 0 {
            return Ok(0);
        }

        let query = format!("SELECT COUNT(*) FROM {table}");
        let count = conn.query_row(query.as_str(), [], |row| row.get::<_, i64>(0))?;
        Ok(count)
    }

    pub(super) fn duckdb_document_count(&self) -> Result<i64> {
        if !self.ctx.duckdb_path.exists() {
            return Ok(0);
        }
        let conn = duckdb::Connection::open(&self.ctx.duckdb_path)
            .with_context(|| format!("opening duckdb at {}", self.ctx.duckdb_path.display()))?;
        let count = conn.query_row(
            "SELECT COUNT(*) FROM knowledge_document_versions",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count)
    }

    pub(super) fn latest_relation(&self) -> Result<Option<RelationAssertionRecord>> {
        if !self.ctx.sqlite_path.exists() {
            return Ok(None);
        }
        let conn = rusqlite::Connection::open(&self.ctx.sqlite_path)
            .with_context(|| format!("opening sqlite db at {}", self.ctx.sqlite_path.display()))?;
        let query = "SELECT source_knowledge_item_version_id, target_type, target_id, relation_type, association_method, provenance_json
                     FROM knowledge_relation_assertions
                     ORDER BY rowid DESC
                     LIMIT 1";
        conn.query_row(query, [], |row| {
            Ok(RelationAssertionRecord {
                source_knowledge_item_version_id: row.get(0)?,
                target_type: row.get(1)?,
                target_id: row.get(2)?,
                relation_type: row.get(3)?,
                association_method: row.get(4)?,
                provenance_json: row.get(5)?,
            })
        })
        .optional()
        .map_err(anyhow::Error::from)
    }

    pub(super) fn all_relations(&self) -> Result<Vec<RelationAssertionRecord>> {
        if !self.ctx.sqlite_path.exists() {
            return Ok(Vec::new());
        }
        let conn = rusqlite::Connection::open(&self.ctx.sqlite_path)
            .with_context(|| format!("opening sqlite db at {}", self.ctx.sqlite_path.display()))?;
        let mut stmt = conn.prepare(
            "SELECT source_knowledge_item_version_id, target_type, target_id, relation_type, association_method, provenance_json
             FROM knowledge_relation_assertions
             ORDER BY rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RelationAssertionRecord {
                source_knowledge_item_version_id: row.get(0)?,
                target_type: row.get(1)?,
                target_id: row.get(2)?,
                relation_type: row.get(3)?,
                association_method: row.get(4)?,
                provenance_json: row.get(5)?,
            })
        })?;
        rows.map(|row| row.map_err(anyhow::Error::from)).collect()
    }

    pub(super) fn source_provenance_rows(&self) -> Result<Vec<String>> {
        self.provenance_rows_from_table("knowledge_sources")
    }

    pub(super) fn item_provenance_rows(&self) -> Result<Vec<String>> {
        self.provenance_rows_from_table("knowledge_items")
    }

    fn provenance_rows_from_table(&self, table: &str) -> Result<Vec<String>> {
        if !self.ctx.sqlite_path.exists() {
            return Ok(Vec::new());
        }
        let conn = rusqlite::Connection::open(&self.ctx.sqlite_path)
            .with_context(|| format!("opening sqlite db at {}", self.ctx.sqlite_path.display()))?;
        let query = format!("SELECT provenance_json FROM {table}");
        let mut stmt = conn.prepare(query.as_str())?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map_err(anyhow::Error::from)).collect()
    }
}

fn test_backends(base: &Path) -> StoreBackendConfig {
    StoreBackendConfig {
        relational: RelationalBackendConfig {
            provider: RelationalProvider::Sqlite,
            sqlite_path: Some(base.join("relational.db").to_string_lossy().to_string()),
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path: Some(base.join("events.duckdb").to_string_lossy().to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
        blobs: BlobStorageConfig {
            provider: BlobStorageProvider::Local,
            local_path: Some(base.join("blobs").to_string_lossy().to_string()),
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        },
    }
}

fn default_provider_config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        github: Some(crate::config::GithubProviderConfig {
            token: "gh-token".to_string(),
        }),
        atlassian: Some(AtlassianProviderConfig {
            site_url: base_url.to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
        }),
        jira: Some(AtlassianProviderConfig {
            site_url: base_url.to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        }),
        confluence: Some(AtlassianProviderConfig {
            site_url: base_url.to_string(),
            email: "confluence@example.com".to_string(),
            token: "confluence-token".to_string(),
        }),
    }
}

fn assert_provider_matches(expected_provider: &str, actual: &KnowledgeProvider) -> Result<()> {
    let expected = normalize_provider_name(expected_provider);
    if expected != actual.as_str() {
        bail!(
            "stub provider mismatch: expected `{expected}`, got `{}`",
            actual.as_str()
        );
    }
    Ok(())
}

fn normalize_provider_name(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "github" => "github",
        "jira" => "jira",
        "confluence" => "confluence",
        other => panic!("unsupported provider label `{other}`"),
    }
}

fn build_record(
    parsed: &crate::engine::devql::capabilities::knowledge::ParsedKnowledgeUrl,
    title: &str,
    body: &str,
    updated_at: &str,
) -> ExternalKnowledgeRecord {
    ExternalKnowledgeRecord {
        provider: parsed.provider.as_str().to_string(),
        source_kind: parsed.source_kind.as_str().to_string(),
        canonical_external_id: parsed.canonical_external_id.clone(),
        canonical_url: parsed.canonical_url.clone(),
        title: title.to_string(),
        state: Some("open".to_string()),
        author: Some("spiros".to_string()),
        updated_at: Some(updated_at.to_string()),
        body_preview: Some(body.to_string()),
        normalized_fields: json!({
            "title": title,
            "updated_at": updated_at,
        }),
        payload: KnowledgePayloadData {
            raw_payload: json!({
                "title": title,
                "body": body,
                "updated_at": updated_at,
            }),
            body_text: Some(body.to_string()),
            body_html: None,
            body_adf: None,
            discussion: None,
        },
    }
}

pub(super) async fn run_add_flow(
    services: &KnowledgeServices,
    ctx: &mut TestRuntimeContext,
    url: &str,
    commit: Option<&str>,
) -> Result<(IngestKnowledgeResult, Option<AssociateKnowledgeResult>)> {
    let ingest_result = services
        .ingestion
        .ingest_source(
            IngestKnowledgeRequest {
                url: url.to_string(),
            },
            ctx,
        )
        .await?;

    let association_result = if let Some(commit_ref) = commit {
        Some(
            services
                .relations
                .associate_to_commit(ctx, &ingest_result, commit_ref)
                .await?,
        )
    } else {
        None
    };

    Ok((ingest_result, association_result))
}

pub(super) async fn run_associate_flow(
    services: &KnowledgeServices,
    ctx: &mut TestRuntimeContext,
    source_ref: &str,
    target_ref: &str,
) -> Result<AssociateKnowledgeResult> {
    services
        .relations
        .associate_by_refs(ctx, source_ref, target_ref)
        .await
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::KnowledgeBddHarness;

    const ISSUE_URL: &str = "https://github.com/bitloops/bitloops/issues/42";

    #[tokio::test]
    async fn support_harness_add_flow_smoke() -> Result<()> {
        let mut harness = KnowledgeBddHarness::new()?;
        harness.stub_success("GitHub", ISSUE_URL, "Issue 42", "Body", None)?;

        let (ingest, association) = harness.add(ISSUE_URL, None).await?;
        assert!(association.is_none());
        assert_eq!(ingest.provider, "github");
        assert_eq!(harness.sqlite_row_count("knowledge_items")?, 1);
        assert_eq!(harness.duckdb_document_count()?, 1);
        assert_eq!(
            harness.sqlite_row_count("knowledge_relation_assertions")?,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn support_harness_associate_flow_smoke() -> Result<()> {
        let mut harness = KnowledgeBddHarness::new()?;
        harness.stub_success("GitHub", ISSUE_URL, "Issue 42", "Body", None)?;

        let (ingest, _) = harness.add(ISSUE_URL, None).await?;
        let association = harness
            .associate(
                &format!("knowledge:{}", ingest.knowledge_item_id),
                "commit:HEAD",
            )
            .await?;

        assert_eq!(association.target_type, "commit");
        assert_eq!(
            harness
                .latest_relation()?
                .expect("relation should exist")
                .target_type,
            "commit"
        );
        Ok(())
    }
}
