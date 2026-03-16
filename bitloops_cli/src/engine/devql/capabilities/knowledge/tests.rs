use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::providers::{
    KnowledgeProviderClient, build_confluence_document, build_github_document,
    build_jira_document,
};
use super::storage::{
    content_hash, knowledge_payload_key, serialize_payload,
};
use super::types::{
    BoxFuture, FetchedKnowledgeDocument, IngestKnowledgeRequest, KnowledgeHostContext,
    KnowledgePayloadData, KnowledgeSourceKind,
};
use super::{KnowledgeCapability, KnowledgePlugin, format_knowledge_add_result};
use crate::store_config::{
    AtlassianProviderConfig, BlobStorageConfig, BlobStorageProvider, EventsBackendConfig,
    EventsProvider, ProviderConfig, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig,
};
use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::resolve_repo_identity;
use crate::engine::devql::capabilities::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
};
use crate::test_support::git_fixtures::{git_ok, init_test_repo};

struct StubClient {
    calls: Arc<AtomicUsize>,
    responses: Mutex<VecDeque<StubResponse>>,
}

enum StubResponse {
    Document(FetchedKnowledgeDocument),
    Error(String),
    BreakSqliteAndDocument(FetchedKnowledgeDocument),
}

impl StubClient {
    fn new(responses: Vec<StubResponse>) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }
}

impl KnowledgeProviderClient for StubClient {
    fn fetch<'a>(
        &'a self,
        _parsed: &'a super::types::ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self
                .responses
                .lock()
                .expect("responses mutex")
                .pop_front()
                .expect("stub response");
            match response {
                StubResponse::Document(document) => Ok(document),
                StubResponse::Error(message) => bail!(message),
                StubResponse::BreakSqliteAndDocument(document) => {
                    let sqlite_path = host.backends.relational.resolve_sqlite_db_path()?;
                    if sqlite_path.exists() {
                        fs::remove_file(&sqlite_path).ok();
                    }
                    fs::create_dir_all(&sqlite_path)?;
                    Ok(document)
                }
            }
        })
    }
}

#[test]
fn github_provider_maps_issue_payload() -> Result<()> {
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")?;
    let document = build_github_document(
        &parsed,
        json!({
            "title": "Issue title",
            "state": "open",
            "updated_at": "2026-03-16T10:00:00Z",
            "body": "Issue body",
            "user": { "login": "spiros" }
        }),
    )?;
    assert_eq!(document.title, "Issue title");
    assert_eq!(document.state.as_deref(), Some("open"));
    assert_eq!(document.author.as_deref(), Some("spiros"));
    Ok(())
}

#[test]
fn github_provider_maps_pull_request_payload() -> Result<()> {
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/pull/1370")?;
    let document = build_github_document(
        &parsed,
        json!({
            "title": "PR title",
            "state": "open",
            "updated_at": "2026-03-16T10:00:00Z",
            "body": "PR body",
            "user": { "login": "spiros" },
            "pull_request": { "url": "https://api.github.com/repos/bitloops/bitloops/pulls/1370" }
        }),
    )?;
    assert_eq!(document.title, "PR title");
    assert_eq!(document.body_preview.as_deref(), Some("PR body"));
    assert_eq!(document.external_id, "github://bitloops/bitloops/pull/1370");
    Ok(())
}

#[test]
fn jira_provider_maps_issue_payload() -> Result<()> {
    let parsed =
        super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;
    let document = build_jira_document(
        &parsed,
        json!({
            "fields": {
                "summary": "Jira title",
                "updated": "2026-03-16T11:00:00Z",
                "status": { "name": "In Progress" },
                "reporter": { "displayName": "Spiros" },
                "description": {
                    "type": "doc",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Jira body" }]
                        }
                    ]
                }
            }
        }),
    )?;
    assert_eq!(document.title, "Jira title");
    assert_eq!(document.author.as_deref(), Some("Spiros"));
    assert_eq!(document.body_preview.as_deref(), Some("Jira body"));
    Ok(())
}

#[test]
fn confluence_provider_maps_page_payload() -> Result<()> {
    let parsed = super::url::parse_knowledge_url(
        "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
    )?;
    let document = build_confluence_document(
        &parsed,
        json!({
            "title": "Knowledge page",
            "version": {
                "when": "2026-03-16T12:00:00Z",
                "by": { "displayName": "Docs User" }
            },
            "body": {
                "storage": {
                    "value": "<p>Hello <strong>world</strong></p>"
                }
            }
        }),
    )?;
    assert_eq!(document.title, "Knowledge page");
    assert_eq!(document.author.as_deref(), Some("Docs User"));
    assert_eq!(document.body_preview.as_deref(), Some("Hello world"));
    Ok(())
}

#[tokio::test]
async fn plugin_persists_repository_scoped_knowledge_and_dispatches_to_github() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let github = StubClient::new(vec![StubResponse::Document(sample_document(
        "Issue one",
        Some("Issue body"),
    ))]);
    let github_calls = github.calls.clone();
    let jira = StubClient::new(vec![]);
    let jira_calls = jira.calls.clone();
    let confluence = StubClient::new(vec![]);
    let confluence_calls = confluence.calls.clone();
    let plugin = KnowledgePlugin::with_clients(
        Box::new(github),
        Box::new(jira),
        Box::new(confluence),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(result.provider, "github");
    assert_eq!(github_calls.load(Ordering::SeqCst), 1);
    assert_eq!(jira_calls.load(Ordering::SeqCst), 0);
    assert_eq!(confluence_calls.load(Ordering::SeqCst), 0);
    assert_eq!(sqlite_row_count(&sqlite_path(&host), "knowledge_items")?, 1);
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 1);
    let blob_path = knowledge_payload_key(
        &host.repo.repo_id,
        &result.knowledge_item_id,
        &result.document_version_id,
    );
    assert!(host.payload_store.payload_exists(&blob_path)?);
    Ok(())
}

#[tokio::test]
async fn plugin_dispatches_to_github_pull_request_handler() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let github = StubClient::new(vec![StubResponse::Document(sample_document(
        "PR one",
        Some("PR body"),
    ))]);
    let github_calls = github.calls.clone();
    let jira = StubClient::new(vec![]);
    let jira_calls = jira.calls.clone();
    let confluence = StubClient::new(vec![]);
    let confluence_calls = confluence.calls.clone();
    let plugin = KnowledgePlugin::with_clients(
        Box::new(github),
        Box::new(jira),
        Box::new(confluence),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/pull/1370".to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(result.provider, "github");
    assert_eq!(result.source_kind, KnowledgeSourceKind::GithubPullRequest.as_str());
    assert_eq!(github_calls.load(Ordering::SeqCst), 1);
    assert_eq!(jira_calls.load(Ordering::SeqCst), 0);
    assert_eq!(confluence_calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn plugin_dispatches_to_jira_handler() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let github = StubClient::new(vec![]);
    let github_calls = github.calls.clone();
    let jira = StubClient::new(vec![StubResponse::Document(sample_document(
        "Jira issue",
        Some("Jira body"),
    ))]);
    let jira_calls = jira.calls.clone();
    let confluence = StubClient::new(vec![]);
    let confluence_calls = confluence.calls.clone();
    let plugin = KnowledgePlugin::with_clients(
        Box::new(github),
        Box::new(jira),
        Box::new(confluence),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://bitloops.atlassian.net/browse/CLI-1370".to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(result.provider, "jira");
    assert_eq!(result.source_kind, KnowledgeSourceKind::JiraIssue.as_str());
    assert_eq!(github_calls.load(Ordering::SeqCst), 0);
    assert_eq!(jira_calls.load(Ordering::SeqCst), 1);
    assert_eq!(confluence_calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn plugin_dispatches_to_confluence_handler() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let github = StubClient::new(vec![]);
    let github_calls = github.calls.clone();
    let jira = StubClient::new(vec![]);
    let jira_calls = jira.calls.clone();
    let confluence = StubClient::new(vec![StubResponse::Document(sample_document(
        "Knowledge page",
        Some("Page body"),
    ))]);
    let confluence_calls = confluence.calls.clone();
    let plugin = KnowledgePlugin::with_clients(
        Box::new(github),
        Box::new(jira),
        Box::new(confluence),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge"
                    .to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(result.provider, "confluence");
    assert_eq!(result.source_kind, KnowledgeSourceKind::ConfluencePage.as_str());
    assert_eq!(github_calls.load(Ordering::SeqCst), 0);
    assert_eq!(jira_calls.load(Ordering::SeqCst), 0);
    assert_eq!(confluence_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn plugin_reuses_item_and_version_for_duplicate_content() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let document = sample_document("Issue one", Some("Issue body"));
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(document.clone()),
            StubResponse::Document(document),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(first.knowledge_item_id, second.knowledge_item_id);
    assert_eq!(first.document_version_id, second.document_version_id);
    assert_eq!(second.item_status, super::types::KnowledgeItemStatus::Reused);
    assert_eq!(second.version_status, super::types::KnowledgeVersionStatus::Reused);
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 1);
    Ok(())
}

#[tokio::test]
async fn plugin_creates_new_version_when_payload_changes() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(sample_document("Issue one", Some("Issue body"))),
            StubResponse::Document(sample_document("Issue one", Some("Updated body"))),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await?;

    assert_eq!(first.knowledge_item_id, second.knowledge_item_id);
    assert_ne!(first.document_version_id, second.document_version_id);
    assert_eq!(second.version_status, super::types::KnowledgeVersionStatus::Created);
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 2);
    Ok(())
}

#[tokio::test]
async fn plugin_creates_commit_relation_when_commit_flag_present() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(sample_document(
            "Issue one",
            Some("Issue body"),
        ))])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: Some(commit_sha.clone()),
            },
        )
        .await?;

    assert!(result.relation_assertion_id.is_some());
    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn plugin_provider_failure_leaves_no_rows() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Error(
            "provider failure".to_string(),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let err = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await
        .expect_err("provider failure must fail");

    assert!(err.to_string().contains("provider failure"));
    assert_eq!(sqlite_row_count(&sqlite_path(&host), "knowledge_items")?, 0);
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 0);
    Ok(())
}

#[tokio::test]
async fn plugin_unsupported_url_leaves_no_rows() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let err = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://example.com/not-supported".to_string(),
                commit: None,
            },
        )
        .await
        .expect_err("unsupported URL must fail");

    assert!(err.to_string().contains("unsupported knowledge URL"));
    assert_eq!(sqlite_row_count(&sqlite_path(&host), "knowledge_items")?, 0);
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 0);
    Ok(())
}

#[tokio::test]
async fn plugin_compensates_blob_and_duckdb_when_sqlite_persist_fails() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::BreakSqliteAndDocument(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let err = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
                commit: None,
            },
        )
        .await
        .expect_err("sqlite failure must fail");

    let rendered = err.to_string().to_ascii_lowercase();
    assert!(rendered.contains("sqlite"));
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 0);
    let expected_path = knowledge_payload_key(
        &host.repo.repo_id,
        &super::storage::knowledge_item_id(
            &host.repo.repo_id,
            &super::storage::knowledge_source_id("github://bitloops/bitloops/issues/42"),
        ),
        &super::storage::document_version_id(
            &super::storage::knowledge_item_id(
                &host.repo.repo_id,
                &super::storage::knowledge_source_id("github://bitloops/bitloops/issues/42"),
            ),
            &content_hash(&serialize_payload(&json!({
                "raw_payload": json!({
                    "source": "stub",
                    "title": "Issue one",
                    "body": "Issue body"
                }),
                "body_text": "Issue body",
                "body_html": Value::Null,
                "body_adf": Value::Null
            }))?),
        ),
    );
    assert!(!host.payload_store.payload_exists(&expected_path)?);
    Ok(())
}

#[test]
fn format_result_renders_expected_summary() {
    let rendered = format_knowledge_add_result(&super::types::IngestKnowledgeResult {
        provider: "github".to_string(),
        source_kind: KnowledgeSourceKind::GithubIssue.as_str().to_string(),
        repo_identity: "local://local/repo".to_string(),
        knowledge_item_id: "item-1".to_string(),
        document_version_id: "version-1".to_string(),
        item_status: super::types::KnowledgeItemStatus::Created,
        version_status: super::types::KnowledgeVersionStatus::Created,
        relation_assertion_id: None,
    });

    assert!(rendered.contains("Knowledge added"));
    assert!(rendered.contains("provider: github"));
    assert!(rendered.contains("association: none"));
}

fn build_test_host(temp: &TempDir, provider_config: ProviderConfig) -> Result<KnowledgeHostContext> {
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root)?;
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops-test@example.com");
    git_ok(&repo_root, &["commit", "--allow-empty", "-m", "initial"]);
    let repo = resolve_repo_identity(&repo_root)?;
    let backends = test_backends(temp);
    let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
    let relational_store =
        SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
    let document_store = DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
    let payload_store = BlobKnowledgePayloadStore::from_backend_config(&repo_root, &backends)?;

    Ok(KnowledgeHostContext {
        repo_root,
        repo,
        backends,
        provider_config,
        relational_store,
        document_store,
        payload_store,
    })
}

fn provider_config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        github: Some(crate::store_config::GithubProviderConfig {
            token: "gh-token".to_string(),
        }),
        jira: Some(AtlassianProviderConfig {
            site_url: base_url.trim_end_matches('/').to_string(),
            email: "jira@example.com".to_string(),
            token: "jira-token".to_string(),
        }),
        confluence: Some(AtlassianProviderConfig {
            site_url: base_url.trim_end_matches('/').to_string(),
            email: "confluence@example.com".to_string(),
            token: "confluence-token".to_string(),
        }),
    }
}

fn test_backends(temp: &TempDir) -> StoreBackendConfig {
    StoreBackendConfig {
        relational: RelationalBackendConfig {
            provider: RelationalProvider::Sqlite,
            sqlite_path: Some(temp.path().join("relational.db").to_string_lossy().to_string()),
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path: Some(temp.path().join("events.duckdb").to_string_lossy().to_string()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        },
        blobs: BlobStorageConfig {
            provider: BlobStorageProvider::Local,
            local_path: Some(temp.path().join("blobs").to_string_lossy().to_string()),
            s3_bucket: None,
            s3_region: None,
            s3_access_key_id: None,
            s3_secret_access_key: None,
            gcs_bucket: None,
            gcs_credentials_path: None,
        },
    }
}

fn sample_document(title: &str, body: Option<&str>) -> FetchedKnowledgeDocument {
    let raw_payload = json!({
        "source": "stub",
        "title": title,
        "body": body.unwrap_or_default(),
    });
    FetchedKnowledgeDocument {
        external_id: "stub://external".to_string(),
        title: title.to_string(),
        web_url: "https://example.com/item".to_string(),
        state: Some("open".to_string()),
        author: Some("spiros".to_string()),
        updated_at: Some("2026-03-16T10:00:00Z".to_string()),
        body_preview: body.map(ToString::to_string),
        normalized_fields: json!({
            "title": title,
            "body": body,
        }),
        payload: KnowledgePayloadData {
            raw_payload,
            body_text: body.map(ToString::to_string),
            body_html: None,
            body_adf: None,
        },
    }
}

fn sqlite_path(host: &KnowledgeHostContext) -> PathBuf {
    host.backends
        .relational
        .resolve_sqlite_db_path()
        .expect("sqlite path")
}

fn duckdb_path(host: &KnowledgeHostContext) -> PathBuf {
    host.backends.events.duckdb_path_or_default()
}

fn sqlite_row_count(path: &Path, table: &str) -> Result<i64> {
    if !path.exists() {
        return Ok(0);
    }
    let conn = rusqlite::Connection::open(path)
        .with_context(|| format!("opening sqlite db at {}", path.display()))?;
    let exists = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
    )?;
    if exists == 0 {
        return Ok(0);
    }
    let query = format!("SELECT COUNT(*) FROM {table}");
    let count = conn.query_row(query.as_str(), [], |row: &rusqlite::Row<'_>| {
        row.get::<_, i64>(0)
    })?;
    Ok(count)
}

fn duckdb_document_count(path: &Path) -> Result<i64> {
    if !path.exists() {
        return Ok(0);
    }
    let conn = duckdb::Connection::open(path)
        .with_context(|| format!("opening duckdb at {}", path.display()))?;
    let count = conn.query_row(
        "SELECT COUNT(*) FROM knowledge_document_versions",
        [],
        |row: &duckdb::Row<'_>| row.get::<_, i64>(0),
    )?;
    Ok(count)
}
