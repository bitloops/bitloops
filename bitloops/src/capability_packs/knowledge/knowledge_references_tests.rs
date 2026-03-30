use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::adapters::connectors::{ConnectorContext, ConnectorRegistry, KnowledgeConnectorAdapter};
use crate::capability_packs::knowledge::storage::{
    KnowledgeDocumentRepository, KnowledgeDocumentVersionRow, KnowledgeItemRow,
    KnowledgeRelationAssertionRow, KnowledgeRelationalRepository, KnowledgeSourceRow,
};
use crate::config::ProviderConfig;
use crate::host::capability_host::config_view::CapabilityConfigView;
use crate::host::capability_host::gateways::{
    BlobPayloadGateway, BlobPayloadRef, ProvenanceBuilder, RelationalGateway,
};
use crate::host::capability_host::{CapabilityIngestContext, KnowledgeIngestContext};
use crate::host::devql::RepoIdentity;
use crate::test_support::git_fixtures::{git_ok, init_test_repo};

use super::*;

const TEST_ARTEFACT_ID: &str = "bbbbbbbb-1111-2222-3333-444444444444";

struct EmptyConnectorRegistry {
    provider_config: ProviderConfig,
}

impl ConnectorContext for EmptyConnectorRegistry {
    fn provider_config(&self) -> &ProviderConfig {
        &self.provider_config
    }
}

impl ConnectorRegistry for EmptyConnectorRegistry {
    fn knowledge_adapter_for(
        &self,
        _parsed: &crate::capability_packs::knowledge::ParsedKnowledgeUrl,
    ) -> Result<&dyn KnowledgeConnectorAdapter> {
        Err(anyhow!(
            "connector lookup should not be called in refs tests"
        ))
    }
}

struct NoopBlobGateway;

impl BlobPayloadGateway for NoopBlobGateway {
    fn write_payload(&self, _key: &str, _bytes: &[u8]) -> Result<BlobPayloadRef> {
        Err(anyhow!("blob writes are not used in refs tests"))
    }

    fn delete_payload(&self, _payload: &BlobPayloadRef) -> Result<()> {
        Ok(())
    }

    fn payload_exists(&self, _storage_path: &str) -> Result<bool> {
        Ok(false)
    }
}

struct NoopProvenance;

impl ProvenanceBuilder for NoopProvenance {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

struct FakeRelationalGateway {
    item: Option<KnowledgeItemRow>,
    source: Option<KnowledgeSourceRow>,
    checkpoint_map: HashMap<String, String>,
    artefacts: HashMap<String, bool>,
}

impl RelationalGateway for FakeRelationalGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, checkpoint_ref: &str) -> Result<String> {
        self.checkpoint_map
            .get(checkpoint_ref)
            .cloned()
            .ok_or_else(|| anyhow!("checkpoint `{checkpoint_ref}` not found"))
    }

    fn artefact_exists(&self, _repo_id: &str, artefact_id: &str) -> Result<bool> {
        Ok(*self.artefacts.get(artefact_id).unwrap_or(&false))
    }

    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String> {
        bail!(
            "FakeRelationalGateway: load_repo_id_for_commit not implemented (commit {commit_sha})"
        )
    }

    fn load_production_artefacts(
        &self,
        commit_sha: &str,
    ) -> Result<Vec<crate::models::ProductionArtefact>> {
        bail!(
            "FakeRelationalGateway: load_production_artefacts not implemented (commit {commit_sha})"
        )
    }

    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        bail!(
            "FakeRelationalGateway: load_artefacts_for_file_lines not implemented (commit {commit_sha}, file {file_path})"
        )
    }
}

impl KnowledgeRelationalRepository for FakeRelationalGateway {
    fn initialise_schema(&self) -> Result<()> {
        Ok(())
    }

    fn persist_ingestion(
        &self,
        _source: &KnowledgeSourceRow,
        _item: &KnowledgeItemRow,
    ) -> Result<()> {
        Ok(())
    }

    fn insert_relation_assertion(&self, _relation: &KnowledgeRelationAssertionRow) -> Result<()> {
        Ok(())
    }

    fn find_item(&self, _repo_id: &str, _source_id: &str) -> Result<Option<KnowledgeItemRow>> {
        Ok(self.item.clone())
    }

    fn find_item_by_id(
        &self,
        _repo_id: &str,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItemRow>> {
        Ok(self
            .item
            .clone()
            .filter(|item| item.knowledge_item_id == knowledge_item_id))
    }

    fn find_source_by_id(&self, knowledge_source_id: &str) -> Result<Option<KnowledgeSourceRow>> {
        Ok(self
            .source
            .clone()
            .filter(|source| source.knowledge_source_id == knowledge_source_id))
    }

    fn list_items_for_repo(&self, _repo_id: &str, _limit: usize) -> Result<Vec<KnowledgeItemRow>> {
        Ok(self.item.clone().into_iter().collect())
    }
}

struct FakeDocumentGateway {
    rows: HashMap<String, KnowledgeDocumentVersionRow>,
}

impl KnowledgeDocumentRepository for FakeDocumentGateway {
    fn initialise_schema(&self) -> Result<()> {
        Ok(())
    }

    fn has_knowledge_item_version(
        &self,
        _knowledge_item_id: &str,
        _content_hash: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn insert_knowledge_item_version(&self, _row: &KnowledgeDocumentVersionRow) -> Result<()> {
        Ok(())
    }

    fn delete_knowledge_item_version(&self, _knowledge_item_version_id: &str) -> Result<()> {
        Ok(())
    }

    fn find_knowledge_item_version(
        &self,
        knowledge_item_version_id: &str,
    ) -> Result<Option<KnowledgeDocumentVersionRow>> {
        Ok(self.rows.get(knowledge_item_version_id).cloned())
    }

    fn list_versions_for_item(
        &self,
        knowledge_item_id: &str,
    ) -> Result<Vec<KnowledgeDocumentVersionRow>> {
        Ok(self
            .rows
            .values()
            .filter(|row| row.knowledge_item_id == knowledge_item_id)
            .cloned()
            .collect())
    }
}

struct RefTestContext {
    repo_root: PathBuf,
    repo: RepoIdentity,
    relational: FakeRelationalGateway,
    documents: FakeDocumentGateway,
    connectors: EmptyConnectorRegistry,
    blobs: NoopBlobGateway,
    provenance: NoopProvenance,
}

impl CapabilityIngestContext for RefTestContext {
    fn repo(&self) -> &RepoIdentity {
        &self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            json!({}),
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

    fn host_relational(&self) -> &dyn RelationalGateway {
        &self.relational
    }
}

impl KnowledgeIngestContext for RefTestContext {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        &self.relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        &self.documents
    }
}

fn test_repo_identity(repo_root: &Path) -> RepoIdentity {
    let identity = repo_root.to_string_lossy().to_string();
    RepoIdentity {
        provider: "local".to_string(),
        organization: "bitloops".to_string(),
        name: "refs-tests".to_string(),
        identity: identity.clone(),
        repo_id: crate::host::devql::deterministic_uuid(&format!("repo://{identity}")),
    }
}

fn build_context(temp: &TempDir) -> Result<(RefTestContext, String)> {
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root)?;
    init_test_repo(&repo_root, "main", "Bitloops Bot", "bot@bitloops.dev");
    fs::write(repo_root.join("README.md"), "# refs\n")?;
    git_ok(repo_root.as_path(), &["add", "."]);
    git_ok(repo_root.as_path(), &["commit", "-m", "initial commit"]);
    let head_sha = git_ok(repo_root.as_path(), &["rev-parse", "HEAD"]);

    let repo = test_repo_identity(repo_root.as_path());
    let knowledge_item_id = "item-1".to_string();
    let knowledge_item_version_id = "version-1".to_string();
    let knowledge_source_id = "source-1".to_string();

    let relational = FakeRelationalGateway {
        item: Some(KnowledgeItemRow {
            knowledge_item_id: knowledge_item_id.clone(),
            repo_id: repo.repo_id.clone(),
            knowledge_source_id: knowledge_source_id.clone(),
            item_kind: "github_issue".to_string(),
            latest_knowledge_item_version_id: knowledge_item_version_id.clone(),
            provenance_json: "{}".to_string(),
        }),
        source: Some(KnowledgeSourceRow {
            knowledge_source_id,
            provider: "github".to_string(),
            source_kind: "github_issue".to_string(),
            canonical_external_id: "github://bitloops/bitloops/issues/42".to_string(),
            canonical_url: "https://github.com/bitloops/bitloops/issues/42".to_string(),
            provenance_json: "{}".to_string(),
        }),
        checkpoint_map: HashMap::from([(
            "checkpoint-short".to_string(),
            "deadbeefcafe".to_string(),
        )]),
        artefacts: HashMap::from([(TEST_ARTEFACT_ID.to_string(), true)]),
    };

    let documents = FakeDocumentGateway {
        rows: HashMap::from([
            (
                knowledge_item_version_id.clone(),
                KnowledgeDocumentVersionRow {
                    knowledge_item_version_id,
                    knowledge_item_id,
                    provider: "github".to_string(),
                    source_kind: "github_issue".to_string(),
                    content_hash: "hash-1".to_string(),
                    title: "Issue 42".to_string(),
                    state: Some("open".to_string()),
                    author: Some("spiros".to_string()),
                    updated_at: Some("2026-03-19T10:00:00Z".to_string()),
                    body_preview: Some("Issue body".to_string()),
                    normalized_fields_json: "{}".to_string(),
                    storage_backend: "local".to_string(),
                    storage_path: "knowledge/repo/item/version/payload.json".to_string(),
                    payload_mime_type: "application/json".to_string(),
                    payload_size_bytes: 10,
                    provenance_json: "{}".to_string(),
                    created_at: Some("2026-03-19T10:00:00Z".to_string()),
                },
            ),
            (
                "version-2".to_string(),
                KnowledgeDocumentVersionRow {
                    knowledge_item_version_id: "version-2".to_string(),
                    knowledge_item_id: "item-2".to_string(),
                    provider: "github".to_string(),
                    source_kind: "github_issue".to_string(),
                    content_hash: "hash-2".to_string(),
                    title: "Issue 2".to_string(),
                    state: Some("open".to_string()),
                    author: Some("spiros".to_string()),
                    updated_at: Some("2026-03-19T10:00:00Z".to_string()),
                    body_preview: Some("Issue body 2".to_string()),
                    normalized_fields_json: "{}".to_string(),
                    storage_backend: "local".to_string(),
                    storage_path: "knowledge/repo/item/version-2/payload.json".to_string(),
                    payload_mime_type: "application/json".to_string(),
                    payload_size_bytes: 10,
                    provenance_json: "{}".to_string(),
                    created_at: Some("2026-03-19T10:00:00Z".to_string()),
                },
            ),
        ]),
    };

    Ok((
        RefTestContext {
            repo_root,
            repo,
            relational,
            documents,
            connectors: EmptyConnectorRegistry {
                provider_config: ProviderConfig::default(),
            },
            blobs: NoopBlobGateway,
            provenance: NoopProvenance,
        },
        head_sha,
    ))
}

#[test]
fn parse_knowledge_ref_handles_supported_kinds() -> Result<()> {
    let parsed_item = parse_knowledge_ref("knowledge:item-1")?;
    assert_eq!(
        parsed_item,
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            knowledge_item_version_id: None,
        }
    );

    let parsed_item_with_version = parse_knowledge_ref("knowledge:item-1:version-2")?;
    assert_eq!(
        parsed_item_with_version,
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            knowledge_item_version_id: Some("version-2".to_string()),
        }
    );

    assert_eq!(
        parse_knowledge_ref("knowledge_version:version-1")?,
        KnowledgeRef::KnowledgeVersion {
            knowledge_item_version_id: "version-1".to_string(),
        }
    );
    assert_eq!(
        parse_knowledge_ref("commit:abc123")?,
        KnowledgeRef::Commit {
            rev: "abc123".to_string(),
        }
    );
    assert_eq!(
        parse_knowledge_ref("checkpoint:deadbeef")?,
        KnowledgeRef::Checkpoint {
            checkpoint_id: "deadbeef".to_string(),
        }
    );
    assert_eq!(
        parse_knowledge_ref("artefact:artefact-1")?,
        KnowledgeRef::Artefact {
            artefact_id: "artefact-1".to_string(),
        }
    );
    Ok(())
}

#[test]
fn parse_knowledge_ref_rejects_invalid_syntax() {
    assert!(parse_knowledge_ref("knowledge").is_err());
    assert!(parse_knowledge_ref("knowledge:").is_err());
    assert!(parse_knowledge_ref("knowledge::version").is_err());
    assert!(parse_knowledge_ref("unknown:value").is_err());
}

#[test]
fn resolve_source_ref_supports_item_and_deprecated_version_refs() -> Result<()> {
    let temp = TempDir::new()?;
    let (ctx, _) = build_context(&temp)?;

    let resolved_item = resolve_source_ref(&ctx, "knowledge:item-1")?;
    assert_eq!(resolved_item.knowledge_item_id, "item-1");
    assert_eq!(resolved_item.source_knowledge_item_version_id, "version-1");

    let resolved_item_with_version = resolve_source_ref(&ctx, "knowledge:item-1:version-1")?;
    assert_eq!(resolved_item_with_version.knowledge_item_id, "item-1");
    assert_eq!(
        resolved_item_with_version.source_knowledge_item_version_id,
        "version-1"
    );

    let resolved_deprecated = resolve_source_ref(&ctx, "knowledge_version:version-1")?;
    assert_eq!(resolved_deprecated.knowledge_item_id, "item-1");
    assert_eq!(
        resolved_deprecated.source_knowledge_item_version_id,
        "version-1"
    );

    assert!(resolve_source_ref(&ctx, "commit:abc123").is_err());
    assert!(resolve_source_ref(&ctx, "checkpoint:deadbeef").is_err());
    assert!(resolve_source_ref(&ctx, "artefact:artefact-1").is_err());

    Ok(())
}

#[test]
fn resolve_source_ref_rejects_missing_item_and_mismatched_versions() -> Result<()> {
    let temp = TempDir::new()?;
    let (mut ctx, _) = build_context(&temp)?;

    let missing_item_err = resolve_source_ref(&ctx, "knowledge:missing-item")
        .expect_err("missing source item must fail");
    assert!(
        missing_item_err
            .to_string()
            .contains("knowledge item `missing-item` not found")
    );

    let mismatched_version_err = resolve_source_ref(&ctx, "knowledge:item-1:version-2")
        .expect_err("mismatched version must fail");
    assert!(
        mismatched_version_err
            .to_string()
            .contains("does not belong")
    );

    let item = ctx
        .relational
        .item
        .as_mut()
        .ok_or_else(|| anyhow!("missing test item"))?;
    item.latest_knowledge_item_version_id = "   ".to_string();

    let missing_latest_err = resolve_source_ref(&ctx, "knowledge:item-1")
        .expect_err("missing latest source version must fail");
    assert!(
        missing_latest_err
            .to_string()
            .contains("has no latest knowledge item version")
    );

    Ok(())
}

#[test]
fn resolve_source_ref_rejects_deprecated_version_when_repo_item_is_missing() -> Result<()> {
    let temp = TempDir::new()?;
    let (mut ctx, _) = build_context(&temp)?;
    ctx.relational.item = None;

    let err = resolve_source_ref(&ctx, "knowledge_version:version-1")
        .expect_err("deprecated version ref without repo item must fail");
    assert!(err.to_string().contains("not found in current repo"));

    Ok(())
}

#[test]
fn resolve_target_ref_supports_commit_knowledge_checkpoint_and_artefact() -> Result<()> {
    let temp = TempDir::new()?;
    let (ctx, head_sha) = build_context(&temp)?;

    let commit = resolve_target_ref(&ctx, "commit:HEAD")?;
    assert_eq!(commit, ResolvedKnowledgeTargetRef::Commit { sha: head_sha });

    let knowledge = resolve_target_ref(&ctx, "knowledge:item-1")?;
    assert_eq!(
        knowledge,
        ResolvedKnowledgeTargetRef::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            target_knowledge_item_version_id: Some("version-1".to_string()),
        }
    );

    let knowledge_versioned = resolve_target_ref(&ctx, "knowledge:item-1:version-1")?;
    assert_eq!(
        knowledge_versioned,
        ResolvedKnowledgeTargetRef::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            target_knowledge_item_version_id: Some("version-1".to_string()),
        }
    );

    let checkpoint = resolve_target_ref(&ctx, "checkpoint:checkpoint-short")?;
    assert_eq!(
        checkpoint,
        ResolvedKnowledgeTargetRef::Checkpoint {
            checkpoint_id: "deadbeefcafe".to_string(),
        }
    );

    let artefact = resolve_target_ref(&ctx, &format!("artefact:{TEST_ARTEFACT_ID}"))?;
    assert_eq!(
        artefact,
        ResolvedKnowledgeTargetRef::Artefact {
            artefact_id: TEST_ARTEFACT_ID.to_string(),
        }
    );

    assert!(resolve_target_ref(&ctx, "knowledge:item-1:missing-version").is_err());
    assert!(resolve_target_ref(&ctx, "knowledge:item-1:version-2").is_err());
    assert!(resolve_target_ref(&ctx, "knowledge_version:version-1").is_err());
    assert!(resolve_target_ref(&ctx, "artefact:missing").is_err());
    assert!(resolve_target_ref(&ctx, "commit:   ").is_err());

    Ok(())
}

#[test]
fn resolve_target_ref_uses_latest_version_for_unversioned_target() -> Result<()> {
    let temp = TempDir::new()?;
    let (mut ctx, _) = build_context(&temp)?;
    let item = ctx
        .relational
        .item
        .as_mut()
        .ok_or_else(|| anyhow!("missing test item"))?;
    item.latest_knowledge_item_version_id = "  version-1  ".to_string();

    let resolved = resolve_target_ref(&ctx, "knowledge:item-1")?;
    assert_eq!(
        resolved,
        ResolvedKnowledgeTargetRef::KnowledgeItem {
            knowledge_item_id: "item-1".to_string(),
            target_knowledge_item_version_id: Some("version-1".to_string()),
        }
    );

    Ok(())
}

#[test]
fn resolve_target_ref_rejects_unsupported_kind_and_invalid_artefact_shape() -> Result<()> {
    let temp = TempDir::new()?;
    let (ctx, _) = build_context(&temp)?;

    let unsupported = resolve_target_ref(&ctx, "knowledge_version:version-1")
        .expect_err("knowledge_version target should remain unsupported");
    assert!(
        unsupported
            .to_string()
            .contains("not supported as a target")
    );

    let invalid_artefact =
        resolve_target_ref(&ctx, "artefact:AAAAAAAA-1111-2222-3333-444444444444")
            .expect_err("uppercase artefact id must fail validation");
    assert!(
        invalid_artefact
            .to_string()
            .contains("expected lowercase UUID")
    );

    Ok(())
}

#[test]
fn resolve_target_ref_rejects_target_without_latest_version() -> Result<()> {
    let temp = TempDir::new()?;
    let (mut ctx, _) = build_context(&temp)?;
    let item = ctx
        .relational
        .item
        .as_mut()
        .ok_or_else(|| anyhow!("missing test item"))?;
    item.latest_knowledge_item_version_id = "   ".to_string();

    let err = resolve_target_ref(&ctx, "knowledge:item-1")
        .expect_err("missing latest target version must fail");
    assert!(
        err.to_string()
            .contains("has no latest knowledge item version")
    );

    Ok(())
}

#[test]
fn is_valid_artefact_id_enforces_lowercase_uuid_shape() {
    assert!(is_valid_artefact_id("aaaaaaaa-1111-2222-3333-444444444444"));
    assert!(!is_valid_artefact_id(
        "AAAAAAAA-1111-2222-3333-444444444444"
    ));
    assert!(!is_valid_artefact_id("bbbbbbbb-1111-2222-3333-44444444444"));
    assert!(!is_valid_artefact_id(
        "bbbbbbbb-1111-2222-3333-4444444444444"
    ));
}

#[test]
fn resolve_commit_sha_rejects_empty_and_invalid_refs() -> Result<()> {
    let temp = TempDir::new()?;
    let (ctx, _) = build_context(&temp)?;

    let empty_err =
        resolve_commit_sha(ctx.repo_root(), "   ").expect_err("whitespace ref must fail");
    assert!(
        empty_err
            .to_string()
            .contains("commit sha must not be empty")
    );

    let invalid_err =
        resolve_commit_sha(ctx.repo_root(), "not-a-real-rev").expect_err("invalid rev should fail");
    assert!(
        invalid_err
            .to_string()
            .contains("validating commit `not-a-real-rev`")
    );

    Ok(())
}
