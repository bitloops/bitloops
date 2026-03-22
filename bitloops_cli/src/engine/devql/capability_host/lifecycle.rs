use anyhow::{Result, bail};

use super::contexts::{CapabilityMigrationContext, KnowledgeMigrationContext};
use super::descriptor::CapabilityDescriptor;
use super::health::{CapabilityHealthCheck, CapabilityHealthResult};
use super::migrations::{CapabilityMigration, MigrationRunner};
use super::registrar::{CapabilityPack, CapabilityRegistrar};

pub fn validate_descriptor(descriptor: &CapabilityDescriptor) -> Result<()> {
    if descriptor.id.trim().is_empty() {
        bail!("[capability_pack:descriptor] id must not be empty");
    }
    if descriptor.display_name.trim().is_empty() {
        bail!(
            "[capability_pack:descriptor] display_name must not be empty (capability_id={})",
            descriptor.id
        );
    }
    if descriptor.version.trim().is_empty() {
        bail!(
            "[capability_pack:descriptor] version must not be empty (capability_id={})",
            descriptor.id
        );
    }
    if descriptor.api_version == 0 {
        bail!(
            "[capability_pack:descriptor] api_version must be > 0 (capability_id={})",
            descriptor.id
        );
    }
    Ok(())
}

pub fn validate_pack(pack: &dyn CapabilityPack) -> Result<()> {
    validate_descriptor(pack.descriptor())
}

pub fn register_pack(
    registrar: &mut dyn CapabilityRegistrar,
    pack: &dyn CapabilityPack,
) -> Result<()> {
    validate_pack(pack)?;
    pack.register(registrar)
}

pub fn run_migrations<M: KnowledgeMigrationContext>(
    migrations: &[CapabilityMigration],
    ctx: &mut M,
) -> Result<()> {
    for migration in migrations {
        match migration.run {
            MigrationRunner::Core(f) => f(ctx as &mut dyn CapabilityMigrationContext)?,
            MigrationRunner::Knowledge(f) => f(ctx as &mut dyn KnowledgeMigrationContext)?,
        }
    }
    Ok(())
}

pub fn run_health_checks(
    capability_id: &str,
    checks: &[CapabilityHealthCheck],
    ctx: &dyn super::contexts::CapabilityHealthContext,
) -> Vec<(String, CapabilityHealthResult)> {
    checks
        .iter()
        .map(|check| (format!("{capability_id}.{}", check.name), (check.run)(ctx)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::knowledge::storage::{
        DuckdbKnowledgeDocumentStore, SqliteKnowledgeRelationalStore,
    };
    use crate::config::ProviderConfig;
    use crate::engine::devql::RepoIdentity;
    use crate::engine::devql::capability_host::config_view::CapabilityConfigView;
    use crate::engine::devql::capability_host::contexts::{
        CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
        CapabilityMigrationContext, KnowledgeMigrationContext,
    };
    use crate::engine::devql::capability_host::gateways::{
        ConnectorRegistry, DocumentStoreGateway, RelationalGateway, StoreHealthGateway,
    };
    use crate::engine::devql::capability_host::health::{
        CapabilityHealthCheck, CapabilityHealthResult,
    };
    use crate::engine::devql::capability_host::migrations::{CapabilityMigration, MigrationRunner};
    use crate::engine::devql::capability_host::registrar::{
        BoxFuture, CapabilityPack, CapabilityRegistrar, IngesterHandler, IngesterRegistration,
        KnowledgeIngesterRegistration, KnowledgeStageRegistration, QueryExample, SchemaModule,
        StageHandler, StageRegistration, StageRequest, StageResponse,
    };
    use crate::engine::devql::capability_host::runtime_contexts::LocalStoreHealthGateway;
    use crate::storage::SqliteConnectionPool;
    use crate::utils::paths;
    use anyhow::Result;
    use serde_json::json;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;

    static VALID_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
        id: "knowledge",
        display_name: "Knowledge",
        version: "1.0.0",
        api_version: 1,
        description: "test capability",
        default_enabled: true,
        experimental: false,
        dependencies: &[],
        required_host_features: &[],
    };

    static INVALID_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
        id: "",
        display_name: "Knowledge",
        version: "1.0.0",
        api_version: 1,
        description: "invalid",
        default_enabled: true,
        experimental: false,
        dependencies: &[],
        required_host_features: &[],
    };

    struct CollectingRegistrar {
        stages: Vec<&'static str>,
        ingesters: Vec<&'static str>,
        schema_modules: Vec<&'static str>,
        query_examples: Vec<&'static str>,
    }

    impl CollectingRegistrar {
        fn new() -> Self {
            Self {
                stages: Vec::new(),
                ingesters: Vec::new(),
                schema_modules: Vec::new(),
                query_examples: Vec::new(),
            }
        }
    }

    impl CapabilityRegistrar for CollectingRegistrar {
        fn register_stage(&mut self, stage: StageRegistration) -> Result<()> {
            self.stages.push(stage.stage_name);
            Ok(())
        }

        fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()> {
            self.ingesters.push(ingester.ingester_name);
            Ok(())
        }

        fn register_knowledge_stage(&mut self, stage: KnowledgeStageRegistration) -> Result<()> {
            self.stages.push(stage.stage_name);
            Ok(())
        }

        fn register_knowledge_ingester(
            &mut self,
            ingester: KnowledgeIngesterRegistration,
        ) -> Result<()> {
            self.ingesters.push(ingester.ingester_name);
            Ok(())
        }

        fn register_schema_module(&mut self, module: SchemaModule) -> Result<()> {
            self.schema_modules.push(module.name);
            Ok(())
        }

        fn register_query_examples(&mut self, examples: &'static [QueryExample]) -> Result<()> {
            self.query_examples
                .extend(examples.iter().map(|example| example.name));
            Ok(())
        }
    }

    struct NoopStageHandler;

    impl StageHandler for NoopStageHandler {
        fn execute<'a>(
            &'a self,
            _request: StageRequest,
            _ctx: &'a mut dyn CapabilityExecutionContext,
        ) -> BoxFuture<'a, Result<StageResponse>> {
            Box::pin(async move { Ok(StageResponse::new(json!({}), "")) })
        }
    }

    struct NoopIngesterHandler;

    impl IngesterHandler for NoopIngesterHandler {
        fn ingest<'a>(
            &'a self,
            _request: crate::engine::devql::capability_host::registrar::IngestRequest,
            _ctx: &'a mut dyn CapabilityIngestContext,
        ) -> BoxFuture<'a, Result<crate::engine::devql::capability_host::registrar::IngestResult>>
        {
            Box::pin(async move {
                Ok(
                    crate::engine::devql::capability_host::registrar::IngestResult::new(
                        json!({}),
                        "",
                    ),
                )
            })
        }
    }

    struct ValidPack {
        called: Arc<AtomicBool>,
    }

    impl ValidPack {
        fn new(called: Arc<AtomicBool>) -> Self {
            Self { called }
        }
    }

    impl CapabilityPack for ValidPack {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &VALID_DESCRIPTOR
        }

        fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
            self.called.store(true, Ordering::SeqCst);
            registrar.register_stage(StageRegistration::new(
                "knowledge",
                "knowledge.stage",
                Arc::new(NoopStageHandler),
            ))?;
            registrar.register_ingester(IngesterRegistration::new(
                "knowledge",
                "knowledge.ingester",
                Arc::new(NoopIngesterHandler),
            ))?;
            registrar.register_schema_module(SchemaModule {
                capability_id: "knowledge",
                name: "knowledge.schema",
                description: "schema",
            })?;
            static QUERY_EXAMPLES: [QueryExample; 1] = [QueryExample {
                capability_id: "knowledge",
                name: "knowledge.example",
                query: "knowledge()",
                description: "example",
            }];
            registrar.register_query_examples(&QUERY_EXAMPLES)?;
            Ok(())
        }
    }

    struct InvalidPack {
        called: Arc<AtomicBool>,
    }

    impl InvalidPack {
        fn new(called: Arc<AtomicBool>) -> Self {
            Self { called }
        }
    }

    impl CapabilityPack for InvalidPack {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &INVALID_DESCRIPTOR
        }

        fn register(&self, _registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    struct MigrationContext {
        _temp: TempDir,
        repo_root: PathBuf,
        repo: RepoIdentity,
        relational: SqliteKnowledgeRelationalStore,
        documents: DuckdbKnowledgeDocumentStore,
    }

    impl CapabilityMigrationContext for MigrationContext {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            &self.repo_root
        }

        fn apply_devql_sqlite_ddl(&self, _sql: &str) -> Result<()> {
            Ok(())
        }
    }

    impl KnowledgeMigrationContext for MigrationContext {
        fn relational(&self) -> &dyn RelationalGateway {
            &self.relational
        }

        fn documents(&self) -> &dyn DocumentStoreGateway {
            &self.documents
        }
    }

    struct HealthContext {
        _temp: TempDir,
        repo_root: PathBuf,
        repo: RepoIdentity,
        connectors: crate::adapters::connectors::BuiltinConnectorRegistry,
        stores: LocalStoreHealthGateway,
    }

    impl CapabilityHealthContext for HealthContext {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            &self.repo_root
        }

        fn config_view(&self, capability_id: &str) -> Result<CapabilityConfigView> {
            Ok(CapabilityConfigView::empty(capability_id))
        }

        fn connectors(&self) -> &dyn ConnectorRegistry {
            &self.connectors
        }

        fn stores(&self) -> &dyn StoreHealthGateway {
            &self.stores
        }
    }

    fn make_repo_root() -> TempDir {
        TempDir::new().expect("temp dir")
    }

    fn make_repo_identity() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops-cli".to_string(),
            identity: "local/bitloops/bitloops-cli".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    fn prepare_repo_root(temp: &TempDir) -> PathBuf {
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(repo_root.join(paths::BITLOOPS_RELATIONAL_STORE_DIR))
            .expect("create relational dir");
        fs::create_dir_all(repo_root.join(paths::BITLOOPS_EVENT_STORE_DIR))
            .expect("create event dir");
        fs::create_dir_all(repo_root.join(paths::BITLOOPS_BLOB_STORE_DIR))
            .expect("create blob dir");
        repo_root
    }

    fn make_migration_context() -> MigrationContext {
        let temp = make_repo_root();
        let repo_root = prepare_repo_root(&temp);
        let relational = SqliteKnowledgeRelationalStore::new(
            SqliteConnectionPool::connect(paths::default_relational_db_path(&repo_root))
                .expect("sqlite pool"),
        );
        let documents =
            DuckdbKnowledgeDocumentStore::new(paths::default_events_db_path(&repo_root));

        MigrationContext {
            _temp: temp,
            repo_root,
            repo: make_repo_identity(),
            relational,
            documents,
        }
    }

    #[test]
    fn validate_descriptor_rejects_missing_core_fields() {
        let mut invalid = VALID_DESCRIPTOR;
        invalid.id = "";
        assert!(validate_descriptor(&invalid).is_err());

        invalid = VALID_DESCRIPTOR;
        invalid.display_name = "";
        assert!(validate_descriptor(&invalid).is_err());

        invalid = VALID_DESCRIPTOR;
        invalid.version = "";
        assert!(validate_descriptor(&invalid).is_err());

        invalid = VALID_DESCRIPTOR;
        invalid.api_version = 0;
        assert!(validate_descriptor(&invalid).is_err());
    }

    #[test]
    fn register_pack_invokes_pack_registration_and_records_items() {
        let called = Arc::new(AtomicBool::new(false));
        let pack = ValidPack::new(Arc::clone(&called));
        let mut registrar = CollectingRegistrar::new();

        register_pack(&mut registrar, &pack).expect("register pack");

        assert!(called.load(Ordering::SeqCst));
        assert_eq!(registrar.stages, vec!["knowledge.stage"]);
        assert_eq!(registrar.ingesters, vec!["knowledge.ingester"]);
        assert_eq!(registrar.schema_modules, vec!["knowledge.schema"]);
        assert_eq!(registrar.query_examples, vec!["knowledge.example"]);
    }

    #[test]
    fn register_pack_rejects_invalid_descriptor_before_registration() {
        let called = Arc::new(AtomicBool::new(false));
        let pack = InvalidPack::new(Arc::clone(&called));
        let mut registrar = CollectingRegistrar::new();

        let err = register_pack(&mut registrar, &pack).expect_err("invalid descriptor");

        let msg = err.to_string();
        assert!(
            msg.contains("[capability_pack:descriptor]") && msg.contains("id must not be empty"),
            "unexpected error: {msg}"
        );
        assert!(!called.load(Ordering::SeqCst));
        assert!(registrar.stages.is_empty());
    }

    #[test]
    fn run_migrations_executes_in_order() {
        fn first(ctx: &mut dyn KnowledgeMigrationContext) -> Result<()> {
            let log_path = ctx.repo_root().join("migrations.log");
            ctx.relational().initialise_schema()?;
            ctx.documents().initialise_schema()?;
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?;
            writeln!(file, "first")?;
            Ok(())
        }

        fn second(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
            let log_path = ctx.repo_root().join("migrations.log");
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?;
            writeln!(file, "second")?;
            Ok(())
        }

        let mut ctx = make_migration_context();
        let migrations = [
            CapabilityMigration {
                capability_id: "knowledge",
                version: "1",
                description: "first",
                run: MigrationRunner::Knowledge(first),
            },
            CapabilityMigration {
                capability_id: "knowledge",
                version: "2",
                description: "second",
                run: MigrationRunner::Core(second),
            },
        ];

        run_migrations(&migrations, &mut ctx).expect("run migrations");

        let log = fs::read_to_string(ctx.repo_root.join("migrations.log")).expect("migration log");
        assert_eq!(log, "first\nsecond\n");
    }

    #[test]
    fn run_health_checks_prefixes_results_and_uses_context() {
        fn healthy(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
            let view = ctx.config_view("knowledge").expect("config view");
            assert_eq!(view.capability_id(), "knowledge");
            assert!(ctx.stores().check_relational().is_ok());
            CapabilityHealthResult::ok("healthy")
        }

        fn unhealthy(_: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
            CapabilityHealthResult::failed("unhealthy", "details")
        }

        let temp = make_repo_root();
        let repo_root = prepare_repo_root(&temp);
        let health = HealthContext {
            _temp: temp,
            repo_root,
            repo: make_repo_identity(),
            connectors: crate::adapters::connectors::BuiltinConnectorRegistry::new(
                ProviderConfig::default(),
            )
            .expect("connector registry"),
            stores: LocalStoreHealthGateway,
        };
        let checks = [
            CapabilityHealthCheck {
                name: "healthy",
                run: healthy,
            },
            CapabilityHealthCheck {
                name: "unhealthy",
                run: unhealthy,
            },
        ];

        let results = run_health_checks("knowledge", &checks, &health);

        assert_eq!(results[0].0, "knowledge.healthy");
        assert!(results[0].1.healthy);
        assert_eq!(results[1].0, "knowledge.unhealthy");
        assert!(!results[1].1.healthy);
        assert_eq!(results[1].1.details.as_deref(), Some("details"));
    }
}
