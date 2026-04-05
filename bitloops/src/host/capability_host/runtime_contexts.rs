use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::adapters::connectors::BuiltinConnectorRegistry;
use crate::adapters::languages::builtin_language_adapter_packs;
use crate::capability_packs::knowledge::storage::{
    BlobKnowledgePayloadStore, DuckdbKnowledgeDocumentStore, KnowledgeDocumentRepository,
    KnowledgeRelationalRepository, SqliteKnowledgeRelationalRepository,
};
use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, open_repository_for_repo,
};
use crate::config::{
    ProviderConfig, StoreBackendConfig, resolve_provider_config_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::host::devql::RelationalStorage;
use crate::host::devql::RepoIdentity;
use crate::host::extension_host::{CoreExtensionHost, LanguagePackResolutionInput};
use crate::host::language_adapter::LanguageAdapterRegistry;
use crate::storage::SqliteConnectionPool;

use super::config_view::CapabilityConfigView;
use super::contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext, KnowledgeExecutionContext, KnowledgeIngestContext,
    KnowledgeMigrationContext,
};
use super::gateways::{
    BlobPayloadGateway, CanonicalGraphGateway, ConnectorContext, ConnectorRegistry,
    LanguageServicesGateway, ProvenanceBuilder, RelationalGateway, SqliteRelationalGateway,
    StoreHealthGateway,
};

pub struct BuiltinLanguageServicesGateway {
    extension_host: &'static CoreExtensionHost,
    registry: &'static LanguageAdapterRegistry,
}

impl LanguageServicesGateway for BuiltinLanguageServicesGateway {
    fn test_supports(
        &self,
    ) -> Vec<std::sync::Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        self.registry.all_test_supports()
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<std::sync::Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        let resolved = self
            .extension_host
            .language_packs()
            .resolve(LanguagePackResolutionInput::for_file_path(relative_path))
            .ok()?;
        self.registry.test_support_for_pack(resolved.pack.id)
    }
}

fn builtin_language_services() -> Result<&'static BuiltinLanguageServicesGateway> {
    static SERVICES: OnceLock<Result<BuiltinLanguageServicesGateway, String>> = OnceLock::new();
    let service = SERVICES.get_or_init(|| {
        let extension_host = CoreExtensionHost::with_builtins().map_err(|err| err.to_string())?;
        let registry = LanguageAdapterRegistry::with_builtins(builtin_language_adapter_packs())
            .map_err(|err| err.to_string())?;
        Ok(BuiltinLanguageServicesGateway {
            extension_host: Box::leak(Box::new(extension_host)),
            registry: Box::leak(Box::new(registry)),
        })
    });

    match service {
        Ok(service) => Ok(service),
        Err(error) => anyhow::bail!("failed to initialise built-in language services: {error}"),
    }
}

pub struct LocalCapabilityRuntimeResources {
    pub repo_root: PathBuf,
    pub repo: RepoIdentity,
    pub config_root: Value,
    pub backends: StoreBackendConfig,
    pub provider_config: ProviderConfig,
    pub relational: SqliteRelationalGateway,
    pub knowledge_relational: SqliteKnowledgeRelationalRepository,
    pub knowledge_documents: DuckdbKnowledgeDocumentStore,
    pub blob_payloads: BlobKnowledgePayloadStore,
    pub connectors: BuiltinConnectorRegistry,
    pub provenance: DefaultProvenanceBuilder,
    pub graph: LocalCanonicalGraphGateway,
    pub stores: LocalStoreHealthGateway,
    pub test_harness: Option<std::sync::Mutex<BitloopsTestHarnessRepository>>,
    pub languages: &'static BuiltinLanguageServicesGateway,
}

impl LocalCapabilityRuntimeResources {
    pub fn new(repo_root: &Path, repo: RepoIdentity) -> Result<Self> {
        let backends = resolve_store_backend_config_for_repo(repo_root)?;
        let provider_config = resolve_provider_config_for_repo(repo_root)?;

        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)?;
        let sqlite_pool = SqliteConnectionPool::connect(sqlite_path)?;
        let relational = SqliteRelationalGateway::new(sqlite_pool.clone());
        let knowledge_relational = SqliteKnowledgeRelationalRepository::new(sqlite_pool);
        let knowledge_documents =
            DuckdbKnowledgeDocumentStore::new(backends.events.duckdb_path_or_default());
        let blob_payloads = BlobKnowledgePayloadStore::from_backend_config(repo_root, &backends)?;
        let connectors = BuiltinConnectorRegistry::new(provider_config.clone())?;

        let config_root = build_capability_config_root(&backends, &provider_config);
        let stores = LocalStoreHealthGateway;
        let test_harness = open_repository_for_repo(repo_root)
            .ok()
            .map(std::sync::Mutex::new);

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            repo,
            config_root,
            backends,
            provider_config,
            relational,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance: DefaultProvenanceBuilder,
            graph: LocalCanonicalGraphGateway,
            stores,
            test_harness,
            languages: builtin_language_services()?,
        })
    }

    pub fn runtime(&self) -> LocalCapabilityRuntime<'_> {
        self.runtime_with_relational(None, None, None)
    }

    pub fn runtime_with_relational<'a>(
        &'a self,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
    ) -> LocalCapabilityRuntime<'a> {
        LocalCapabilityRuntime::new(
            &self.repo_root,
            &self.repo,
            &self.config_root,
            &self.backends,
            &self.relational,
            &self.knowledge_relational,
            &self.knowledge_documents,
            &self.blob_payloads,
            &self.connectors,
            &self.provenance,
            &self.graph,
            &self.stores,
            self.test_harness.as_ref(),
            self.languages,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
        )
    }
}

fn build_capability_config_root(
    backends: &StoreBackendConfig,
    providers: &ProviderConfig,
) -> Value {
    serde_json::json!({
        "knowledge": {
            "providers": {
                "github": providers.github.as_ref().map(|_| serde_json::json!({ "configured": true })),
                "jira": providers.jira.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "confluence": providers.confluence.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
                "atlassian": providers.atlassian.as_ref().map(|cfg| serde_json::json!({ "site_url": cfg.site_url })),
            },
            "backends": {
                "relational": if backends.relational.has_postgres() { "postgres" } else { "sqlite" },
                "events": if backends.events.has_clickhouse() { "clickhouse" } else { "duckdb" },
            }
        },
        "host": {
            "invocation": {
                "stage_timeout_secs": 120,
                "ingester_timeout_secs": 300,
                "subquery_timeout_secs": 60
            },
            "cross_pack_access": []
        }
    })
}

pub struct LocalCapabilityRuntime<'a> {
    repo_root: &'a Path,
    repo: &'a RepoIdentity,
    config_root: &'a Value,
    backends: &'a StoreBackendConfig,
    relational: &'a dyn RelationalGateway,
    knowledge_relational: &'a dyn KnowledgeRelationalRepository,
    knowledge_documents: &'a dyn KnowledgeDocumentRepository,
    blob_payloads: &'a dyn BlobPayloadGateway,
    connectors: &'a dyn ConnectorRegistry,
    provenance: &'a dyn ProvenanceBuilder,
    graph: &'a dyn CanonicalGraphGateway,
    stores: &'a dyn StoreHealthGateway,
    test_harness: Option<&'a std::sync::Mutex<BitloopsTestHarnessRepository>>,
    languages: &'a BuiltinLanguageServicesGateway,
    devql_relational: Option<&'a RelationalStorage>,
    invoking_capability_id: Option<&'a str>,
    invoking_ingester_id: Option<&'a str>,
}

impl<'a> LocalCapabilityRuntime<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: &'a Path,
        repo: &'a RepoIdentity,
        config_root: &'a Value,
        backends: &'a StoreBackendConfig,
        relational: &'a dyn RelationalGateway,
        knowledge_relational: &'a dyn KnowledgeRelationalRepository,
        knowledge_documents: &'a dyn KnowledgeDocumentRepository,
        blob_payloads: &'a dyn BlobPayloadGateway,
        connectors: &'a dyn ConnectorRegistry,
        provenance: &'a dyn ProvenanceBuilder,
        graph: &'a dyn CanonicalGraphGateway,
        stores: &'a dyn StoreHealthGateway,
        test_harness: Option<&'a std::sync::Mutex<BitloopsTestHarnessRepository>>,
        languages: &'a BuiltinLanguageServicesGateway,
        devql_relational: Option<&'a RelationalStorage>,
        invoking_capability_id: Option<&'a str>,
        invoking_ingester_id: Option<&'a str>,
    ) -> Self {
        Self {
            repo_root,
            repo,
            config_root,
            backends,
            relational,
            knowledge_relational,
            knowledge_documents,
            blob_payloads,
            connectors,
            provenance,
            graph,
            stores,
            test_harness,
            languages,
            devql_relational,
            invoking_capability_id,
            invoking_ingester_id,
        }
    }
}

impl CapabilityExecutionContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn graph(&self) -> &dyn CanonicalGraphGateway {
        self.graph
    }

    fn host_relational(&self) -> &dyn RelationalGateway {
        self.relational
    }

    fn languages(&self) -> &dyn LanguageServicesGateway {
        self.languages
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        self.test_harness
    }
}

impl CapabilityIngestContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn config_view(&self, capability_id: &str) -> anyhow::Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn blob_payloads(&self) -> &dyn BlobPayloadGateway {
        self.blob_payloads
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        self.connectors
    }

    fn connector_context(&self) -> &dyn ConnectorContext {
        self.connectors
    }

    fn provenance(&self) -> &dyn ProvenanceBuilder {
        self.provenance
    }

    fn host_relational(&self) -> &dyn RelationalGateway {
        self.relational
    }

    fn languages(&self) -> &dyn LanguageServicesGateway {
        self.languages
    }

    fn test_harness_store(&self) -> Option<&std::sync::Mutex<BitloopsTestHarnessRepository>> {
        self.test_harness
    }

    fn clone_edges_rebuild_relational(&self) -> Result<&RelationalStorage> {
        let Some(relational) = self.devql_relational else {
            anyhow::bail!("clone-edge rebuild relational store is not attached to this ingest");
        };
        Ok(relational)
    }

    fn devql_relational(&self) -> Option<&RelationalStorage> {
        self.devql_relational
    }

    fn invoking_capability_id(&self) -> Option<&str> {
        self.invoking_capability_id
    }

    fn invoking_ingester_id(&self) -> Option<&str> {
        self.invoking_ingester_id
    }
}

impl CapabilityMigrationContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn apply_devql_sqlite_ddl(&self, sql: &str) -> Result<()> {
        if self.backends.relational.has_postgres() {
            return Ok(());
        }
        let path = self
            .backends
            .relational
            .resolve_sqlite_db_path_for_repo(self.repo_root)
            .context("resolving SQLite path for DevQL relational DDL")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        )
        .with_context(|| format!("opening SQLite at {}", path.display()))?;
        conn.execute_batch(sql)
            .context("applying DevQL SQLite DDL")?;
        Ok(())
    }
}

impl KnowledgeExecutionContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        self.knowledge_documents
    }
}

impl KnowledgeIngestContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        self.knowledge_documents
    }
}

impl KnowledgeMigrationContext for LocalCapabilityRuntime<'_> {
    fn knowledge_relational(&self) -> &dyn KnowledgeRelationalRepository {
        self.knowledge_relational
    }

    fn knowledge_documents(&self) -> &dyn KnowledgeDocumentRepository {
        self.knowledge_documents
    }
}

impl CapabilityHealthContext for LocalCapabilityRuntime<'_> {
    fn repo(&self) -> &RepoIdentity {
        self.repo
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
    }

    fn config_view(&self, capability_id: &str) -> anyhow::Result<CapabilityConfigView> {
        Ok(CapabilityConfigView::new(
            capability_id.to_string(),
            self.config_root.clone(),
        ))
    }

    fn connectors(&self) -> &dyn ConnectorRegistry {
        self.connectors
    }

    fn stores(&self) -> &dyn StoreHealthGateway {
        self.stores
    }
}

pub struct LocalCanonicalGraphGateway;

impl CanonicalGraphGateway for LocalCanonicalGraphGateway {}

pub struct DefaultProvenanceBuilder;

impl ProvenanceBuilder for DefaultProvenanceBuilder {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
        serde_json::json!({
            "capability": capability_id,
            "operation": operation,
            "details": details,
        })
    }
}

pub struct LocalStoreHealthGateway;

impl StoreHealthGateway for LocalStoreHealthGateway {
    fn check_relational(&self) -> Result<()> {
        Ok(())
    }

    fn check_documents(&self) -> Result<()> {
        Ok(())
    }

    fn check_blobs(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::connectors::{
        ConnectorContext, ConnectorRegistry, KnowledgeConnectorAdapter,
    };
    use crate::capability_packs::knowledge::storage::{
        KnowledgeDocumentRepository, KnowledgeItemRow, KnowledgeRelationAssertionRow,
        KnowledgeRelationalRepository, KnowledgeSourceRow,
    };
    use crate::config::{
        AtlassianProviderConfig, BlobStorageConfig, EventsBackendConfig, GithubProviderConfig,
        RelationalBackendConfig,
    };
    use crate::host::capability_host::contexts::{
        CapabilityHealthContext, CapabilityIngestContext, KnowledgeExecutionContext,
        KnowledgeIngestContext,
    };
    use crate::host::devql::RepoIdentity;
    use anyhow::{Result, bail};
    use serde_json::json;
    use tempfile::tempdir;

    struct DummyGraph;

    impl CanonicalGraphGateway for DummyGraph {}

    struct DummyProvenance;

    impl ProvenanceBuilder for DummyProvenance {
        fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value {
            json!({
                "capability": capability_id,
                "operation": operation,
                "details": details,
            })
        }
    }

    struct DummyStores;

    impl StoreHealthGateway for DummyStores {
        fn check_relational(&self) -> Result<()> {
            Ok(())
        }

        fn check_documents(&self) -> Result<()> {
            Ok(())
        }

        fn check_blobs(&self) -> Result<()> {
            Ok(())
        }
    }

    struct DummyBlobPayloads;

    impl BlobPayloadGateway for DummyBlobPayloads {
        fn write_payload(
            &self,
            _key: &str,
            _bytes: &[u8],
        ) -> Result<crate::host::capability_host::gateways::BlobPayloadRef> {
            bail!("blob payload writes are not used in runtime_contexts tests")
        }

        fn delete_payload(
            &self,
            _payload: &crate::host::capability_host::gateways::BlobPayloadRef,
        ) -> Result<()> {
            Ok(())
        }

        fn payload_exists(&self, _storage_path: &str) -> Result<bool> {
            Ok(false)
        }
    }

    struct DummyConnectorRegistry {
        provider_config: ProviderConfig,
    }

    impl ConnectorContext for DummyConnectorRegistry {
        fn provider_config(&self) -> &ProviderConfig {
            &self.provider_config
        }
    }

    impl ConnectorRegistry for DummyConnectorRegistry {
        fn knowledge_adapter_for(
            &self,
            _parsed: &crate::capability_packs::knowledge::ParsedKnowledgeUrl,
        ) -> Result<&dyn KnowledgeConnectorAdapter> {
            bail!("knowledge adapter lookup is not used in runtime_contexts tests")
        }
    }

    struct DummyRelationalGateway;

    impl RelationalGateway for DummyRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!("resolve_checkpoint_id is not used in runtime_contexts tests")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            Ok(false)
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!("load_repo_id_for_commit is not used in runtime_contexts tests")
        }

        fn load_production_artefacts(
            &self,
            _commit_sha: &str,
        ) -> Result<Vec<crate::models::ProductionArtefact>> {
            Ok(Vec::new())
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            Ok(Vec::new())
        }
    }

    struct DummyKnowledgeRelationalRepository;

    impl KnowledgeRelationalRepository for DummyKnowledgeRelationalRepository {
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

        fn insert_relation_assertion(
            &self,
            _relation: &KnowledgeRelationAssertionRow,
        ) -> Result<()> {
            Ok(())
        }

        fn find_item(&self, _repo_id: &str, _source_id: &str) -> Result<Option<KnowledgeItemRow>> {
            Ok(None)
        }

        fn find_item_by_id(
            &self,
            _repo_id: &str,
            _knowledge_item_id: &str,
        ) -> Result<Option<KnowledgeItemRow>> {
            Ok(None)
        }

        fn find_source_by_id(
            &self,
            _knowledge_source_id: &str,
        ) -> Result<Option<KnowledgeSourceRow>> {
            Ok(None)
        }

        fn list_items_for_repo(
            &self,
            _repo_id: &str,
            _limit: usize,
        ) -> Result<Vec<KnowledgeItemRow>> {
            Ok(Vec::new())
        }
    }

    struct DummyKnowledgeDocumentRepository;

    impl KnowledgeDocumentRepository for DummyKnowledgeDocumentRepository {
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

        fn insert_knowledge_item_version(
            &self,
            _row: &crate::capability_packs::knowledge::storage::KnowledgeDocumentVersionRow,
        ) -> Result<()> {
            Ok(())
        }

        fn delete_knowledge_item_version(&self, _knowledge_item_version_id: &str) -> Result<()> {
            Ok(())
        }

        fn find_knowledge_item_version(
            &self,
            _knowledge_item_version_id: &str,
        ) -> Result<Option<crate::capability_packs::knowledge::storage::KnowledgeDocumentVersionRow>>
        {
            Ok(None)
        }

        fn list_versions_for_item(
            &self,
            _knowledge_item_id: &str,
        ) -> Result<Vec<crate::capability_packs::knowledge::storage::KnowledgeDocumentVersionRow>>
        {
            Ok(Vec::new())
        }
    }

    fn test_repo_identity(repo_root: &Path) -> RepoIdentity {
        let identity = repo_root.to_string_lossy().to_string();
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "runtime-context-tests".to_string(),
            identity: identity.clone(),
            repo_id: crate::host::devql::deterministic_uuid(&format!("repo://{identity}")),
        }
    }

    fn sqlite_backends(repo_root: &Path) -> StoreBackendConfig {
        StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: Some(".bitloops/devql.sqlite".to_string()),
                postgres_dsn: None,
            },
            events: EventsBackendConfig {
                duckdb_path: Some(".bitloops/events.duckdb".to_string()),
                clickhouse_url: None,
                clickhouse_user: None,
                clickhouse_password: None,
                clickhouse_database: None,
            },
            blobs: BlobStorageConfig {
                local_path: Some(
                    repo_root
                        .join(".bitloops/blob")
                        .to_string_lossy()
                        .to_string(),
                ),
                s3_bucket: None,
                s3_region: None,
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        }
    }

    fn postgres_backends(repo_root: &Path) -> StoreBackendConfig {
        StoreBackendConfig {
            relational: RelationalBackendConfig {
                sqlite_path: Some(".bitloops/devql.sqlite".to_string()),
                postgres_dsn: Some("postgres://localhost:5432/bitloops".to_string()),
            },
            events: EventsBackendConfig {
                duckdb_path: Some(".bitloops/events.duckdb".to_string()),
                clickhouse_url: Some("http://localhost:8123".to_string()),
                clickhouse_user: Some("user".to_string()),
                clickhouse_password: Some("secret".to_string()),
                clickhouse_database: Some("analytics".to_string()),
            },
            blobs: BlobStorageConfig {
                local_path: Some(
                    repo_root
                        .join(".bitloops/blob")
                        .to_string_lossy()
                        .to_string(),
                ),
                s3_bucket: None,
                s3_region: None,
                s3_access_key_id: None,
                s3_secret_access_key: None,
                gcs_bucket: None,
                gcs_credentials_path: None,
            },
        }
    }

    #[test]
    fn build_capability_config_root_uses_sqlite_duckdb_labels() {
        let temp = tempdir().expect("tempdir");
        let backends = sqlite_backends(temp.path());
        let root = build_capability_config_root(&backends, &ProviderConfig::default());

        assert_eq!(root["knowledge"]["backends"]["relational"], json!("sqlite"));
        assert_eq!(root["knowledge"]["backends"]["events"], json!("duckdb"));
    }

    #[test]
    fn build_capability_config_root_uses_postgres_clickhouse_labels() {
        let temp = tempdir().expect("tempdir");
        let backends = postgres_backends(temp.path());
        let root = build_capability_config_root(&backends, &ProviderConfig::default());

        assert_eq!(
            root["knowledge"]["backends"]["relational"],
            json!("postgres")
        );
        assert_eq!(root["knowledge"]["backends"]["events"], json!("clickhouse"));
    }

    #[test]
    fn runtime_exposes_repo_repo_root_and_config_view() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let provider_config = ProviderConfig {
            github: Some(GithubProviderConfig {
                token: "token".to_string(),
            }),
            atlassian: Some(AtlassianProviderConfig {
                site_url: "https://example.atlassian.net".to_string(),
                email: "bot@example.com".to_string(),
                token: "token".to_string(),
            }),
            jira: None,
            confluence: None,
        };
        let config_root = json!({
            "capability-a": { "enabled": true },
            "other": { "value": 7 }
        });
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: provider_config.clone(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        assert_eq!(
            CapabilityExecutionContext::repo(&runtime).repo_id,
            repo.repo_id
        );
        assert_eq!(CapabilityExecutionContext::repo_root(&runtime), repo_root);
        let config_view =
            CapabilityIngestContext::config_view(&runtime, "capability-a").expect("config view");
        assert_eq!(config_view.capability_id(), "capability-a");
        assert_eq!(config_view.root()["capability-a"]["enabled"], json!(true));
    }

    #[test]
    fn apply_devql_sqlite_ddl_noops_when_postgres_configured() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = postgres_backends(repo_root);
        let provider_config = ProviderConfig::default();
        let config_root = json!({});
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: provider_config.clone(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        let sqlite_path = repo_root.join(".bitloops/devql.sqlite");
        assert!(!sqlite_path.exists());

        runtime
            .apply_devql_sqlite_ddl("CREATE TABLE should_not_exist (id INTEGER PRIMARY KEY);")
            .expect("postgres mode should not error");

        assert!(!sqlite_path.exists());
    }

    #[test]
    fn apply_devql_sqlite_ddl_creates_and_executes_sqlite_ddl() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let provider_config = ProviderConfig::default();
        let config_root = json!({});
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: provider_config.clone(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        let sqlite_path = repo_root.join(".bitloops/devql.sqlite");
        runtime
            .apply_devql_sqlite_ddl(
                "CREATE TABLE runtime_contexts_test_table (id INTEGER PRIMARY KEY);",
            )
            .expect("sqlite ddl should apply");

        assert!(sqlite_path.exists());
        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite");
        let table_exists = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runtime_contexts_test_table')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("query sqlite_master");
        assert_eq!(table_exists, 1);
    }

    #[test]
    fn default_provenance_builder_wraps_details() {
        let builder = DefaultProvenanceBuilder;
        let value = builder.build("knowledge", "ingest", json!({ "id": 1 }));

        assert_eq!(value["capability"], json!("knowledge"));
        assert_eq!(value["operation"], json!("ingest"));
        assert_eq!(value["details"]["id"], json!(1));
    }

    #[test]
    fn local_store_health_gateway_returns_ok() {
        let gateway = LocalStoreHealthGateway;

        assert!(gateway.check_relational().is_ok());
        assert!(gateway.check_documents().is_ok());
        assert!(gateway.check_blobs().is_ok());
    }

    #[test]
    fn clone_edges_rebuild_relational_requires_devql_attachment() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let config_root = json!({});
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: ProviderConfig::default(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        let err = CapabilityIngestContext::clone_edges_rebuild_relational(&runtime)
            .expect_err("expected missing devql relational");
        assert!(
            err.to_string()
                .contains("clone-edge rebuild relational store is not attached")
        );
        assert!(CapabilityIngestContext::devql_relational(&runtime).is_none());
    }

    #[test]
    fn ingest_context_exposes_invoking_capability_and_ingester_ids() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let config_root = json!({});
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: ProviderConfig::default(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            Some("cap:knowledge"),
            Some("ingest:clone"),
        );

        assert_eq!(
            CapabilityIngestContext::invoking_capability_id(&runtime),
            Some("cap:knowledge")
        );
        assert_eq!(
            CapabilityIngestContext::invoking_ingester_id(&runtime),
            Some("ingest:clone")
        );
    }

    #[test]
    fn health_context_config_view_reads_capability_slice() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let config_root = json!({ "health-cap": { "ok": true } });
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: ProviderConfig::default(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        let view = CapabilityHealthContext::config_view(&runtime, "health-cap").expect("view");
        assert_eq!(view.capability_id(), "health-cap");
        assert_eq!(view.root()["health-cap"]["ok"], json!(true));
    }

    #[test]
    fn knowledge_contexts_delegate_to_dummy_repositories() {
        let temp = tempdir().expect("tempdir");
        let repo_root = temp.path();
        let repo = test_repo_identity(repo_root);
        let backends = sqlite_backends(repo_root);
        let config_root = json!({});
        let relational = DummyRelationalGateway;
        let knowledge_relational = DummyKnowledgeRelationalRepository;
        let knowledge_documents = DummyKnowledgeDocumentRepository;
        let blob_payloads = DummyBlobPayloads;
        let connectors = DummyConnectorRegistry {
            provider_config: ProviderConfig::default(),
        };
        let provenance = DummyProvenance;
        let graph = DummyGraph;
        let stores = DummyStores;
        let languages = builtin_language_services().expect("built-in language services");
        let runtime = LocalCapabilityRuntime::new(
            repo_root,
            &repo,
            &config_root,
            &backends,
            &relational,
            &knowledge_relational,
            &knowledge_documents,
            &blob_payloads,
            &connectors,
            &provenance,
            &graph,
            &stores,
            None,
            languages,
            None,
            None,
            None,
        );

        assert!(KnowledgeExecutionContext::knowledge_relational(&runtime)
            .initialise_schema()
            .is_ok());
        assert!(KnowledgeExecutionContext::knowledge_documents(&runtime)
            .initialise_schema()
            .is_ok());
        assert!(KnowledgeIngestContext::knowledge_relational(&runtime)
            .list_items_for_repo(&repo.repo_id, 5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn builtin_language_gateway_resolves_rust_test_support() {
        let languages = builtin_language_services().expect("built-in language services");
        let support = languages.resolve_test_support_for_path("src/lib.rs");
        assert!(
            support.is_some(),
            "expected Rust language pack to expose test support for .rs paths"
        );
    }
}
