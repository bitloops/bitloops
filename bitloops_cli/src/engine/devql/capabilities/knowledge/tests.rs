use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::plugin::{build_host_context, run_add_flow, run_associate_flow};
use super::providers::{
    ConfluenceKnowledgeClient, GitHubKnowledgeClient, JiraKnowledgeClient, KnowledgeProviderClient,
    build_confluence_document, build_github_document, build_jira_document,
};
use super::refs::{ResolvedKnowledgeTargetRef, resolve_source_ref, resolve_target_ref};
use super::storage::{content_hash, knowledge_payload_key, serialize_payload};
use super::types::{
    AssociateKnowledgeRequest, AssociateKnowledgeResult, BoxFuture, FetchedKnowledgeDocument,
    IngestKnowledgeRequest, KnowledgeAssociationTarget, KnowledgeHostContext, KnowledgePayloadData,
    KnowledgeSourceKind,
};
use super::{
    KnowledgeCapability, KnowledgePlugin, format_knowledge_add_result,
    format_knowledge_associate_result, run_add_command, run_associate_command,
};
use crate::engine::db::SqliteConnectionPool;
use crate::engine::devql::capabilities::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
};
use crate::engine::devql::resolve_repo_identity;
use crate::store_config::{
    AtlassianProviderConfig, BlobStorageConfig, BlobStorageProvider, EventsBackendConfig,
    EventsProvider, ProviderConfig, RelationalBackendConfig, RelationalProvider,
    StoreBackendConfig,
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
fn github_provider_rejects_issue_url_that_resolves_to_pull_request_payload() {
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")
        .expect("parse github issue url");

    let err = build_github_document(
        &parsed,
        json!({
            "title": "PR payload",
            "pull_request": { "url": "https://api.github.com/repos/bitloops/bitloops/pulls/42" }
        }),
    )
    .expect_err("issue url must reject pull request payload");

    assert!(err.to_string().contains("pull request payload"));
}

#[test]
fn jira_provider_maps_issue_payload() -> Result<()> {
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;
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
fn jira_provider_collects_plain_string_description() -> Result<()> {
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;
    let document = build_jira_document(
        &parsed,
        json!({
            "fields": {
                "summary": "Jira title",
                "description": "Plain Jira body"
            }
        }),
    )?;

    assert_eq!(document.body_preview.as_deref(), Some("Plain Jira body"));
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

#[test]
fn knowledge_plugin_builtin_constructs_real_clients() -> Result<()> {
    let _plugin = KnowledgePlugin::builtin()?;
    Ok(())
}

#[tokio::test]
async fn github_client_fetch_requires_provider_config() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, ProviderConfig::default())?;
    let client = GitHubKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("missing github config must fail");

    assert!(err.to_string().contains("knowledge.providers.github"));
    Ok(())
}

#[test]
fn github_client_build_request_includes_required_headers() -> Result<()> {
    let client = GitHubKnowledgeClient::new()?;
    let request = client.build_request(
        "https://api.github.com/repos/bitloops/bitloops/issues/42",
        "gh-token",
    )?;

    assert_eq!(
        request.headers().get(reqwest::header::USER_AGENT),
        Some(&reqwest::header::HeaderValue::from_static("bitloops-cli"))
    );
    assert_eq!(
        request.headers().get(reqwest::header::ACCEPT),
        Some(&reqwest::header::HeaderValue::from_static(
            "application/vnd.github+json"
        ))
    );
    let authorization = request
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .expect("authorization header");
    assert_eq!(
        authorization.to_str().expect("auth header str"),
        "Bearer gh-token"
    );
    Ok(())
}

#[tokio::test]
async fn github_client_rejects_non_github_locator() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let client = GitHubKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("non github locator must fail");

    assert!(err.to_string().contains("non-GitHub locator"));
    Ok(())
}

#[tokio::test]
async fn jira_client_fetch_requires_provider_config() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.atlassian = None;
    config.jira = None;
    let host = build_test_host(&temp, config)?;
    let client = JiraKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("missing jira config must fail");

    assert!(err.to_string().contains("knowledge.providers.jira"));
    assert!(err.to_string().contains("knowledge.providers.atlassian"));
    Ok(())
}

#[tokio::test]
async fn jira_client_fetch_uses_shared_atlassian_provider_config() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.jira = None;
    let host = build_test_host(&temp, config)?;
    let client = JiraKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("network should fail after shared Atlassian config is resolved");

    assert!(!err.to_string().contains("missing Atlassian configuration"));
    Ok(())
}

#[tokio::test]
async fn jira_client_fetch_rejects_site_mismatch_before_network() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://other.atlassian.net"))?;
    let client = JiraKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("site mismatch must fail");

    assert!(err.to_string().contains("does not match configured"));
    Ok(())
}

#[tokio::test]
async fn jira_client_prefers_product_override_over_shared_atlassian() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.jira = Some(AtlassianProviderConfig {
        site_url: "https://override.atlassian.net".to_string(),
        email: "jira-override@example.com".to_string(),
        token: "jira-override-token".to_string(),
    });
    let host = build_test_host(&temp, config)?;
    let client = JiraKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("product-specific override should win and trigger mismatch");

    assert!(err.to_string().contains("override.atlassian.net"));
    Ok(())
}

#[tokio::test]
async fn jira_client_rejects_non_jira_locator() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let client = JiraKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("non jira locator must fail");

    assert!(err.to_string().contains("non-Jira locator"));
    Ok(())
}

#[tokio::test]
async fn confluence_client_fetch_requires_provider_config() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.atlassian = None;
    config.confluence = None;
    let host = build_test_host(&temp, config)?;
    let client = ConfluenceKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url(
        "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
    )?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("missing confluence config must fail");

    assert!(err.to_string().contains("knowledge.providers.confluence"));
    assert!(err.to_string().contains("knowledge.providers.atlassian"));
    Ok(())
}

#[tokio::test]
async fn confluence_client_fetch_uses_shared_atlassian_provider_config() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.confluence = None;
    let host = build_test_host(&temp, config)?;
    let client = ConfluenceKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url(
        "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
    )?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("network should fail after shared Atlassian config is resolved");

    assert!(!err.to_string().contains("missing Atlassian configuration"));
    Ok(())
}

#[tokio::test]
async fn confluence_client_fetch_rejects_site_mismatch_before_network() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://other.atlassian.net"))?;
    let client = ConfluenceKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url(
        "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
    )?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("site mismatch must fail");

    assert!(err.to_string().contains("does not match configured"));
    Ok(())
}

#[tokio::test]
async fn confluence_client_prefers_product_override_over_shared_atlassian() -> Result<()> {
    let temp = TempDir::new()?;
    let mut config = provider_config("https://bitloops.atlassian.net");
    config.confluence = Some(AtlassianProviderConfig {
        site_url: "https://override.atlassian.net".to_string(),
        email: "docs-override@example.com".to_string(),
        token: "docs-override-token".to_string(),
    });
    let host = build_test_host(&temp, config)?;
    let client = ConfluenceKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url(
        "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
    )?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("product-specific override should win and trigger mismatch");

    assert!(err.to_string().contains("override.atlassian.net"));
    Ok(())
}

#[tokio::test]
async fn confluence_client_rejects_non_confluence_locator() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let client = ConfluenceKnowledgeClient::new()?;
    let parsed = super::url::parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")?;

    let err = client
        .fetch(&parsed, &host)
        .await
        .expect_err("non confluence locator must fail");

    assert!(err.to_string().contains("non-Confluence locator"));
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
    let plugin =
        KnowledgePlugin::with_clients(Box::new(github), Box::new(jira), Box::new(confluence));

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
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
    let plugin =
        KnowledgePlugin::with_clients(Box::new(github), Box::new(jira), Box::new(confluence));

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/pull/1370".to_string(),
            },
        )
        .await?;

    assert_eq!(result.provider, "github");
    assert_eq!(
        result.source_kind,
        KnowledgeSourceKind::GithubPullRequest.as_str()
    );
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
    let plugin =
        KnowledgePlugin::with_clients(Box::new(github), Box::new(jira), Box::new(confluence));

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://bitloops.atlassian.net/browse/CLI-1370".to_string(),
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
    let plugin =
        KnowledgePlugin::with_clients(Box::new(github), Box::new(jira), Box::new(confluence));

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge"
                    .to_string(),
            },
        )
        .await?;

    assert_eq!(result.provider, "confluence");
    assert_eq!(
        result.source_kind,
        KnowledgeSourceKind::ConfluencePage.as_str()
    );
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
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    assert_eq!(first.knowledge_item_id, second.knowledge_item_id);
    assert_eq!(first.document_version_id, second.document_version_id);
    assert_eq!(
        second.item_status,
        super::types::KnowledgeItemStatus::Reused
    );
    assert_eq!(
        second.version_status,
        super::types::KnowledgeVersionStatus::Reused
    );
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
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    assert_eq!(first.knowledge_item_id, second.knowledge_item_id);
    assert_ne!(first.document_version_id, second.document_version_id);
    assert_eq!(
        second.version_status,
        super::types::KnowledgeVersionStatus::Created
    );
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 2);
    Ok(())
}

#[tokio::test]
async fn ingest_source_does_not_create_commit_relation_assertion() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        0
    );
    assert!(!result.knowledge_item_id.is_empty());
    Ok(())
}

#[tokio::test]
async fn associate_creates_commit_relation_with_manual_attachment() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    let association_result = plugin
        .associate(
            &host,
            AssociateKnowledgeRequest {
                knowledge_item_id: ingest_result.knowledge_item_id.clone(),
                source_document_version_id: ingest_result.document_version_id.clone(),
                target: KnowledgeAssociationTarget::Commit {
                    sha: commit_sha.clone(),
                },
                relation_type: "associated_with".to_string(),
                association_method: "manual_attachment".to_string(),
                command: "bitloops devql knowledge associate".to_string(),
            },
        )
        .await?;

    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        1
    );
    let relation = sqlite_relation_assertion(&sqlite_path(&host))?.expect("relation assertion");
    assert_eq!(relation.association_method, "manual_attachment");
    assert_eq!(
        relation.source_document_version_id,
        ingest_result.document_version_id
    );
    assert_eq!(relation.target_id, commit_sha);
    assert_eq!(association_result.association_method, "manual_attachment");
    Ok(())
}

#[tokio::test]
async fn associate_is_idempotent_for_same_source_version_and_target() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let request = AssociateKnowledgeRequest {
        knowledge_item_id: ingest_result.knowledge_item_id.clone(),
        source_document_version_id: ingest_result.document_version_id.clone(),
        target: KnowledgeAssociationTarget::Commit {
            sha: commit_sha.clone(),
        },
        relation_type: "associated_with".to_string(),
        association_method: "manual_attachment".to_string(),
        command: "bitloops devql knowledge associate".to_string(),
    };

    let first = plugin.associate(&host, request.clone()).await?;
    let second = plugin.associate(&host, request).await?;

    assert_eq!(first.relation_assertion_id, second.relation_assertion_id);
    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_rejects_invalid_commit() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    let err = run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", ingest_result.knowledge_item_id),
        "commit:does-not-exist",
    )
        .await
        .expect_err("invalid commit must fail");

    assert!(!err.to_string().trim().is_empty());
    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn run_add_flow_orchestrates_ingest_then_associate_for_commit() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(
            sample_document("Issue one", Some("Issue body")),
        )])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let (ingest_result, association_result) = run_add_flow(
        &plugin,
        &host,
        "https://github.com/bitloops/bitloops/issues/42",
        Some(commit_sha.as_str()),
    )
    .await?;

    let association_result = association_result.expect("association result");
    let relation = sqlite_relation_assertion(&sqlite_path(&host))?.expect("relation assertion");
    assert_eq!(
        relation.source_document_version_id,
        ingest_result.document_version_id
    );
    assert_eq!(relation.target_id, commit_sha);
    assert_eq!(association_result.association_method, "manual_attachment");
    assert_eq!(association_result.relation_type, "associated_with");
    Ok(())
}

#[tokio::test]
async fn resolve_source_ref_uses_latest_document_version_for_knowledge_item() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(sample_document("Issue one", Some("Issue body v1"))),
            StubResponse::Document(sample_document("Issue one", Some("Issue body v2"))),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    assert_eq!(first.knowledge_item_id, second.knowledge_item_id);
    assert_ne!(first.document_version_id, second.document_version_id);

    let resolved = resolve_source_ref(&host, &format!("knowledge:{}", first.knowledge_item_id))?;
    assert_eq!(resolved.knowledge_item_id, first.knowledge_item_id);
    assert_eq!(resolved.source_document_version_id, second.document_version_id);
    Ok(())
}

#[tokio::test]
async fn resolve_source_ref_uses_explicit_document_version_for_knowledge_version() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(sample_document("Issue one", Some("Issue body v1"))),
            StubResponse::Document(sample_document("Issue one", Some("Issue body v2"))),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    let resolved = resolve_source_ref(
        &host,
        &format!("knowledge_version:{}", first.document_version_id),
    )?;
    assert_eq!(resolved.knowledge_item_id, first.knowledge_item_id);
    assert_eq!(resolved.source_document_version_id, first.document_version_id);
    assert_ne!(resolved.source_document_version_id, second.document_version_id);
    Ok(())
}

#[tokio::test]
async fn resolve_source_ref_rejects_missing_knowledge_item() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;

    let err = resolve_source_ref(&host, "knowledge:missing-item")
        .expect_err("missing knowledge item must fail");

    assert!(!err.to_string().trim().is_empty());
    Ok(())
}

#[tokio::test]
async fn resolve_source_ref_rejects_missing_document_version() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;

    let err = resolve_source_ref(&host, "knowledge_version:missing-version")
        .expect_err("missing knowledge version must fail");

    assert!(!err.to_string().trim().is_empty());
    Ok(())
}

#[tokio::test]
async fn resolve_source_ref_rejects_commit_as_source() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);

    let err = resolve_source_ref(&host, &format!("commit:{commit_sha}"))
        .expect_err("commit source must fail");

    assert!(err.to_string().contains("cannot be used"));
    Ok(())
}

#[tokio::test]
async fn resolve_target_ref_canonicalizes_commit_sha() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let full_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let short_sha = full_sha.chars().take(7).collect::<String>();

    let resolved = resolve_target_ref(&host, &format!("commit:{short_sha}"))?;
    assert_eq!(
        resolved,
        ResolvedKnowledgeTargetRef::Commit {
            sha: full_sha.clone()
        }
    );
    Ok(())
}

#[tokio::test]
async fn resolve_target_ref_rejects_knowledge_item_target() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;

    let err = resolve_target_ref(&host, "knowledge:item-1")
        .expect_err("knowledge item target must fail");

    assert!(err.to_string().contains("not supported"));
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_creates_commit_relation_from_knowledge_ref() -> Result<()> {
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

    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    let association_result = run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", ingest_result.knowledge_item_id),
        &format!("commit:{commit_sha}"),
    )
    .await?;

    let relation = sqlite_relation_assertion(&sqlite_path(&host))?.expect("relation assertion");
    assert_eq!(relation.source_document_version_id, ingest_result.document_version_id);
    assert_eq!(relation.target_id, commit_sha);
    assert_eq!(relation.association_method, "manual_attachment");
    assert!(relation.provenance_json.contains("\"command\":\"bitloops devql knowledge associate\""));
    assert_eq!(association_result.relation_type, "associated_with");
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_rejects_missing_knowledge_item_before_persisting_rows() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let err = run_associate_flow(
        &plugin,
        &host,
        "knowledge:missing-item",
        &format!("commit:{commit_sha}"),
    )
    .await
    .expect_err("missing source must fail");

    assert!(!err.to_string().trim().is_empty());
    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_creates_commit_relation_from_knowledge_version_ref() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(sample_document("Issue one", Some("Issue body v1"))),
            StubResponse::Document(sample_document("Issue one", Some("Issue body v2"))),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let _second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge_version:{}", first.document_version_id),
        &format!("commit:{commit_sha}"),
    )
    .await?;

    let relation = sqlite_relation_assertion(&sqlite_path(&host))?.expect("relation assertion");
    assert_eq!(relation.source_document_version_id, first.document_version_id);
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_allows_same_item_to_multiple_commits() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let first_commit = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    git_ok(host.repo_root.as_path(), &["commit", "--allow-empty", "-m", "second"]);
    let second_commit = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(sample_document(
            "Issue one",
            Some("Issue body"),
        ))])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;

    run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", ingest_result.knowledge_item_id),
        &format!("commit:{first_commit}"),
    )
    .await?;
    run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", ingest_result.knowledge_item_id),
        &format!("commit:{second_commit}"),
    )
    .await?;

    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        2
    );
    Ok(())
}

#[tokio::test]
async fn run_associate_flow_allows_multiple_items_to_same_commit() -> Result<()> {
    let temp = TempDir::new()?;
    let host = build_test_host(&temp, provider_config("https://bitloops.atlassian.net"))?;
    let commit_sha = git_ok(host.repo_root.as_path(), &["rev-parse", "HEAD"]);
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![
            StubResponse::Document(sample_document("Issue one", Some("Issue body one"))),
            StubResponse::Document(sample_document("Issue two", Some("Issue body two"))),
        ])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );

    let first = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let second = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/43".to_string(),
            },
        )
        .await?;

    run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", first.knowledge_item_id),
        &format!("commit:{commit_sha}"),
    )
    .await?;
    run_associate_flow(
        &plugin,
        &host,
        &format!("knowledge:{}", second.knowledge_item_id),
        &format!("commit:{commit_sha}"),
    )
    .await?;

    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        2
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
    let ingest = super::types::IngestKnowledgeResult {
        provider: "github".to_string(),
        source_kind: KnowledgeSourceKind::GithubIssue.as_str().to_string(),
        repo_identity: "local://local/repo".to_string(),
        knowledge_item_id: "item-1".to_string(),
        document_version_id: "version-1".to_string(),
        item_status: super::types::KnowledgeItemStatus::Created,
        version_status: super::types::KnowledgeVersionStatus::Created,
    };
    let rendered = format_knowledge_add_result(&ingest, None);

    assert!(rendered.contains("Knowledge added"));
    assert!(rendered.contains("provider: github"));
    assert!(rendered.contains("Association: none"));
}

#[test]
fn format_result_renders_association_summary() {
    let ingest = super::types::IngestKnowledgeResult {
        provider: "github".to_string(),
        source_kind: KnowledgeSourceKind::GithubIssue.as_str().to_string(),
        repo_identity: "local://local/repo".to_string(),
        knowledge_item_id: "item-1".to_string(),
        document_version_id: "version-1".to_string(),
        item_status: super::types::KnowledgeItemStatus::Created,
        version_status: super::types::KnowledgeVersionStatus::Created,
    };
    let association = AssociateKnowledgeResult {
        relation_assertion_id: "relation-1".to_string(),
        target_type: "commit".to_string(),
        target_id: "abc123".to_string(),
        relation_type: "associated_with".to_string(),
        association_method: "manual_attachment".to_string(),
    };
    let rendered = format_knowledge_add_result(&ingest, Some(&association));

    assert!(rendered.contains("Association created"));
    assert!(rendered.contains("relation assertion: relation-1"));
    assert!(rendered.contains("method: manual_attachment"));
}

#[test]
fn format_associate_result_renders_expected_summary() {
    let association = AssociateKnowledgeResult {
        relation_assertion_id: "relation-1".to_string(),
        target_type: "commit".to_string(),
        target_id: "abc123".to_string(),
        relation_type: "associated_with".to_string(),
        association_method: "manual_attachment".to_string(),
    };
    let rendered = format_knowledge_associate_result(&association);

    assert!(rendered.contains("Knowledge associated"));
    assert!(rendered.contains("target: commit:abc123"));
    assert!(rendered.contains("method: manual_attachment"));
}

#[test]
fn build_host_context_reads_repo_config() -> Result<()> {
    let temp = TempDir::new()?;
    let repo_root = init_knowledge_repo(&temp)?;
    let repo = resolve_repo_identity(&repo_root)?;
    write_repo_config(
        &repo_root,
        &test_backends(&temp),
        &provider_config("https://bitloops.atlassian.net"),
    )?;

    let host = build_host_context(&repo_root, &repo)?;

    assert_eq!(host.repo.identity, repo.identity);
    assert!(host.provider_config.github.is_some());
    assert!(host.provider_config.atlassian.is_some());
    assert!(host.provider_config.jira.is_some());
    assert!(host.provider_config.confluence.is_some());
    Ok(())
}

#[tokio::test]
async fn run_add_command_returns_missing_provider_error_without_repo_config() -> Result<()> {
    let temp = TempDir::new()?;
    let repo_root = init_knowledge_repo(&temp)?;
    let repo = resolve_repo_identity(&repo_root)?;

    let err = run_add_command(
        &repo_root,
        &repo,
        "https://github.com/bitloops/bitloops/issues/42",
        None,
    )
    .await
    .expect_err("missing provider config must fail");

    assert!(!err.to_string().trim().is_empty());
    Ok(())
}

#[tokio::test]
async fn run_add_command_invalid_commit_fails_before_persisting_rows() -> Result<()> {
    let temp = TempDir::new()?;
    let repo_root = init_knowledge_repo(&temp)?;
    let repo = resolve_repo_identity(&repo_root)?;
    write_repo_config(
        &repo_root,
        &test_backends(&temp),
        &provider_config("https://bitloops.atlassian.net"),
    )?;

    let err = run_add_command(
        &repo_root,
        &repo,
        "https://github.com/bitloops/bitloops/issues/42",
        Some("does-not-exist"),
    )
    .await
    .expect_err("invalid commit must fail before ingestion");

    assert!(!err.to_string().trim().is_empty());

    let host = build_host_context(&repo_root, &repo)?;
    assert_eq!(sqlite_row_count(&sqlite_path(&host), "knowledge_items")?, 0);
    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        0
    );
    assert_eq!(duckdb_document_count(&duckdb_path(&host))?, 0);
    Ok(())
}

#[tokio::test]
async fn run_associate_command_creates_relation_assertion() -> Result<()> {
    let temp = TempDir::new()?;
    let repo_root = init_knowledge_repo(&temp)?;
    let repo = resolve_repo_identity(&repo_root)?;
    write_repo_config(
        &repo_root,
        &test_backends(&temp),
        &provider_config("https://bitloops.atlassian.net"),
    )?;
    let host = build_host_context(&repo_root, &repo)?;
    let plugin = KnowledgePlugin::with_clients(
        Box::new(StubClient::new(vec![StubResponse::Document(sample_document(
            "Issue one",
            Some("Issue body"),
        ))])),
        Box::new(StubClient::new(vec![])),
        Box::new(StubClient::new(vec![])),
    );
    let ingest_result = plugin
        .ingest_source(
            &host,
            IngestKnowledgeRequest {
                url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            },
        )
        .await?;
    let commit_sha = git_ok(repo_root.as_path(), &["rev-parse", "HEAD"]);

    run_associate_command(
        &repo_root,
        &repo,
        &format!("knowledge:{}", ingest_result.knowledge_item_id),
        &format!("commit:{commit_sha}"),
    )
    .await?;

    assert_eq!(
        sqlite_row_count(&sqlite_path(&host), "knowledge_relation_assertions")?,
        1
    );
    Ok(())
}

fn build_test_host(
    temp: &TempDir,
    provider_config: ProviderConfig,
) -> Result<KnowledgeHostContext> {
    let repo_root = init_knowledge_repo(temp)?;
    let repo = resolve_repo_identity(&repo_root)?;
    let backends = test_backends(temp);
    let sqlite_path = backends.relational.resolve_sqlite_db_path()?;
    let relational_store =
        SqliteKnowledgeRelationalStore::new(SqliteConnectionPool::connect(sqlite_path)?);
    let document_store =
        DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
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

fn init_knowledge_repo(temp: &TempDir) -> Result<PathBuf> {
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root)?;
    init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    git_ok(&repo_root, &["commit", "--allow-empty", "-m", "initial"]);
    Ok(repo_root)
}

fn write_repo_config(
    repo_root: &Path,
    backends: &StoreBackendConfig,
    provider_config: &ProviderConfig,
) -> Result<()> {
    let sqlite_path = backends
        .relational
        .sqlite_path
        .as_ref()
        .context("missing sqlite path for test config")?;
    let duckdb_path = backends
        .events
        .duckdb_path
        .as_ref()
        .context("missing duckdb path for test config")?;
    let blob_path = backends
        .blobs
        .local_path
        .as_ref()
        .context("missing blob path for test config")?;
    let github = provider_config
        .github
        .as_ref()
        .context("missing github config for test config")?;
    let atlassian = provider_config
        .atlassian
        .as_ref()
        .context("missing atlassian config for test config")?;
    let jira = provider_config
        .jira_config()
        .context("missing jira config for test config")?;
    let confluence = provider_config
        .confluence_config()
        .context("missing confluence config for test config")?;
    let config_dir = repo_root.join(".bitloops");
    fs::create_dir_all(&config_dir)?;
    fs::write(
        config_dir.join("config.json"),
        json!({
            "stores": {
                "relational": {
                    "provider": "sqlite",
                    "sqlite_path": sqlite_path,
                },
                "events": {
                    "provider": "duckdb",
                    "duckdb_path": duckdb_path,
                },
                "blobs": {
                    "provider": "local",
                    "local_path": blob_path,
                }
            },
            "knowledge": {
                "providers": {
                    "github": {
                        "token": github.token,
                    },
                    "atlassian": {
                        "site_url": atlassian.site_url,
                        "email": atlassian.email,
                        "token": atlassian.token,
                    },
                    "jira": {
                        "site_url": jira.site_url,
                        "email": jira.email,
                        "token": jira.token,
                    },
                    "confluence": {
                        "site_url": confluence.site_url,
                        "email": confluence.email,
                        "token": confluence.token,
                    }
                }
            }
        })
        .to_string(),
    )?;
    Ok(())
}

fn provider_config(base_url: &str) -> ProviderConfig {
    ProviderConfig {
        github: Some(crate::store_config::GithubProviderConfig {
            token: "gh-token".to_string(),
        }),
        atlassian: Some(AtlassianProviderConfig {
            site_url: base_url.trim_end_matches('/').to_string(),
            email: "shared@example.com".to_string(),
            token: "shared-token".to_string(),
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
            sqlite_path: Some(
                temp.path()
                    .join("relational.db")
                    .to_string_lossy()
                    .to_string(),
            ),
            postgres_dsn: None,
        },
        events: EventsBackendConfig {
            provider: EventsProvider::DuckDb,
            duckdb_path: Some(
                temp.path()
                    .join("events.duckdb")
                    .to_string_lossy()
                    .to_string(),
            ),
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

struct RelationAssertionRecord {
    source_document_version_id: String,
    target_id: String,
    association_method: String,
    provenance_json: String,
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

fn sqlite_relation_assertion(path: &Path) -> Result<Option<RelationAssertionRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = rusqlite::Connection::open(path)
        .with_context(|| format!("opening sqlite db at {}", path.display()))?;
    let exists = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'knowledge_relation_assertions'",
        [],
        |row: &rusqlite::Row<'_>| row.get::<_, i64>(0),
    )?;
    if exists == 0 {
        return Ok(None);
    }

    conn.query_row(
        "SELECT source_document_version_id, target_id, association_method, provenance_json
         FROM knowledge_relation_assertions
         LIMIT 1",
        [],
        |row: &rusqlite::Row<'_>| {
            Ok(RelationAssertionRecord {
                source_document_version_id: row.get(0)?,
                target_id: row.get(1)?,
                association_method: row.get(2)?,
                provenance_json: row.get(3)?,
            })
        },
    )
    .map(Some)
    .or_else(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other.into()),
    })
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
