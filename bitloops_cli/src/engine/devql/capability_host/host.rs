use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::engine::devql::RepoIdentity;
use crate::engine::devql::capabilities;

use super::config_view::CapabilityConfigView;
use super::descriptor::CapabilityDescriptor;
use super::health::{CapabilityHealthCheck, CapabilityHealthResult};
use super::lifecycle;
use super::migrations::CapabilityMigration;
use super::registrar::{
    CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration, QueryExample, SchemaModule, StageHandler, StageRegistration,
    StageRequest, StageResponse,
};
use super::runtime_contexts::LocalCapabilityRuntimeResources;

pub struct DevqlCapabilityHost {
    runtime: LocalCapabilityRuntimeResources,
    descriptors: HashMap<String, &'static CapabilityDescriptor>,
    stages: HashMap<(String, String), Arc<dyn StageHandler>>,
    ingesters: HashMap<(String, String), Arc<dyn IngesterHandler>>,
    schema_modules: Vec<SchemaModule>,
    query_examples: Vec<&'static [QueryExample]>,
    migrations: Vec<CapabilityMigration>,
    health_checks: HashMap<String, Vec<CapabilityHealthCheck>>,
    migrations_applied: bool,
}

impl DevqlCapabilityHost {
    pub fn new(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let runtime = LocalCapabilityRuntimeResources::new(&repo_root, repo)?;
        Ok(Self {
            runtime,
            descriptors: HashMap::new(),
            stages: HashMap::new(),
            ingesters: HashMap::new(),
            schema_modules: Vec::new(),
            query_examples: Vec::new(),
            migrations: Vec::new(),
            health_checks: HashMap::new(),
            migrations_applied: false,
        })
    }

    pub fn builtin(repo_root: impl Into<PathBuf>, repo: RepoIdentity) -> Result<Self> {
        let mut host = Self::new(repo_root.into(), repo)?;
        let packs = capabilities::builtin_packs()?;
        host.register_builtin_packs(packs)?;
        Ok(host)
    }

    pub fn repo_root(&self) -> &Path {
        self.runtime.repo_root.as_path()
    }

    pub fn repo(&self) -> &RepoIdentity {
        &self.runtime.repo
    }

    pub fn config_view(&self, capability_id: &str) -> CapabilityConfigView {
        CapabilityConfigView::new(capability_id.to_string(), self.runtime.config_root.clone())
    }

    pub fn register_pack(&mut self, pack: &dyn CapabilityPack) -> Result<()> {
        lifecycle::register_pack(self, pack)?;
        let descriptor = pack.descriptor();
        self.descriptors.insert(descriptor.id.to_string(), descriptor);
        self.migrations.extend_from_slice(pack.migrations());
        self.health_checks
            .entry(descriptor.id.to_string())
            .or_default()
            .extend_from_slice(pack.health_checks());
        Ok(())
    }

    pub fn register_builtin_packs(&mut self, packs: Vec<Box<dyn CapabilityPack>>) -> Result<()> {
        for pack in packs {
            self.register_pack(pack.as_ref())?;
        }
        Ok(())
    }

    pub fn descriptor(&self, capability_id: &str) -> Option<&'static CapabilityDescriptor> {
        self.descriptors.get(capability_id).copied()
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &'static CapabilityDescriptor> + '_ {
        self.descriptors.values().copied()
    }

    pub fn schema_modules(&self) -> &[SchemaModule] {
        self.schema_modules.as_slice()
    }

    pub fn query_examples(&self) -> &[&'static [QueryExample]] {
        self.query_examples.as_slice()
    }

    pub async fn invoke_ingester(
        &mut self,
        capability_id: &str,
        ingester_name: &str,
        payload: Value,
    ) -> Result<IngestResult> {
        self.ensure_migrations_applied()?;

        let handler = self
            .ingesters
            .get(&(capability_id.to_string(), ingester_name.to_string()))
            .cloned();
        let Some(handler) = handler else {
            bail!(
                "ingester `{ingester_name}` is not registered for capability `{capability_id}`"
            );
        };

        let request = IngestRequest::new(payload);
        let mut runtime = self.runtime.runtime();
        handler.ingest(request, &mut runtime).await
    }

    pub async fn invoke_stage(
        &mut self,
        capability_id: &str,
        stage_name: &str,
        payload: Value,
    ) -> Result<StageResponse> {
        self.ensure_migrations_applied()?;

        let handler = self
            .stages
            .get(&(capability_id.to_string(), stage_name.to_string()))
            .cloned();
        let Some(handler) = handler else {
            bail!("stage `{stage_name}` is not registered for capability `{capability_id}`");
        };

        let request = StageRequest::new(payload);
        let mut runtime = self.runtime.runtime();
        handler.execute(request, &mut runtime).await
    }

    pub fn run_health_checks(&self, capability_id: &str) -> Vec<(String, CapabilityHealthResult)> {
        let checks = self
            .health_checks
            .get(capability_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let runtime = self.runtime.runtime();
        lifecycle::run_health_checks(capability_id, checks, &runtime)
    }

    pub fn has_stage(&self, capability_id: &str, stage_name: &str) -> bool {
        self.stages
            .contains_key(&(capability_id.to_string(), stage_name.to_string()))
    }

    fn ensure_migrations_applied(&mut self) -> Result<()> {
        if self.migrations_applied {
            return Ok(());
        }
        let mut runtime = self.runtime.runtime();
        lifecycle::run_migrations(&self.migrations, &mut runtime)?;
        self.migrations_applied = true;
        Ok(())
    }
}

impl CapabilityRegistrar for DevqlCapabilityHost {
    fn register_stage(&mut self, stage: StageRegistration) -> Result<()> {
        let key = (stage.capability_id.to_string(), stage.stage_name.to_string());
        if self.stages.contains_key(&key) {
            bail!(
                "stage `{}` is already registered for capability `{}`",
                stage.stage_name,
                stage.capability_id
            );
        }
        self.stages.insert(key, stage.handler);
        Ok(())
    }

    fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()> {
        let key = (
            ingester.capability_id.to_string(),
            ingester.ingester_name.to_string(),
        );
        if self.ingesters.contains_key(&key) {
            bail!(
                "ingester `{}` is already registered for capability `{}`",
                ingester.ingester_name,
                ingester.capability_id
            );
        }
        self.ingesters.insert(key, ingester.handler);
        Ok(())
    }

    fn register_schema_module(&mut self, module: SchemaModule) -> Result<()> {
        self.schema_modules.push(module);
        Ok(())
    }

    fn register_query_examples(&mut self, examples: &'static [QueryExample]) -> Result<()> {
        self.query_examples.push(examples);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::devql::RepoIdentity;
    use crate::engine::devql::capability_host::contexts::{
        CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
        CapabilityMigrationContext,
    };
    use crate::engine::devql::capability_host::health::{
        CapabilityHealthCheck, CapabilityHealthResult,
    };
    use crate::engine::devql::capability_host::migrations::CapabilityMigration;
    use crate::engine::devql::capability_host::registrar::{
        BoxFuture, CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult,
        IngesterHandler, IngesterRegistration, QueryExample, SchemaModule, StageHandler,
        StageRegistration, StageRequest, StageResponse,
    };
    use crate::engine::paths;
    use anyhow::Result;
    use serde::Deserialize;
    use serde_json::json;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;

    static TEST_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
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

    #[derive(Debug, Deserialize, PartialEq)]
    struct IngestInput {
        value: String,
    }

    struct FullStageHandler;

    impl StageHandler for FullStageHandler {
        fn execute<'a>(
            &'a self,
            request: StageRequest,
            ctx: &'a mut dyn CapabilityExecutionContext,
        ) -> BoxFuture<'a, Result<StageResponse>> {
            Box::pin(async move {
                Ok(StageResponse::json(json!({
                    "limit": request.limit(),
                    "repo_root": ctx.repo_root().display().to_string(),
                })))
            })
        }
    }

    struct FullIngesterHandler;

    impl IngesterHandler for FullIngesterHandler {
        fn ingest<'a>(
            &'a self,
            request: IngestRequest,
            ctx: &'a mut dyn CapabilityIngestContext,
        ) -> BoxFuture<'a, Result<IngestResult>> {
            Box::pin(async move {
                let input: IngestInput = request.parse_json()?;
                Ok(IngestResult::new(
                    json!({
                        "value": input.value,
                        "repo_root": ctx.repo_root().display().to_string(),
                    }),
                    format!("ingested {}", input.value),
                ))
            })
        }
    }

    struct StageOnlyHandler;

    impl StageHandler for StageOnlyHandler {
        fn execute<'a>(
            &'a self,
            _request: StageRequest,
            _ctx: &'a mut dyn CapabilityExecutionContext,
        ) -> BoxFuture<'a, Result<StageResponse>> {
            Box::pin(async move { Ok(StageResponse::new(json!({}), "stage")) })
        }
    }

    struct IngesterOnlyHandler;

    impl IngesterHandler for IngesterOnlyHandler {
        fn ingest<'a>(
            &'a self,
            _request: IngestRequest,
            _ctx: &'a mut dyn CapabilityIngestContext,
        ) -> BoxFuture<'a, Result<IngestResult>> {
            Box::pin(async move { Ok(IngestResult::new(json!({}), "ingest")) })
        }
    }

    struct FullPack {
        called: Arc<AtomicBool>,
    }

    impl FullPack {
        fn new(called: Arc<AtomicBool>) -> Self {
            Self { called }
        }
    }

    impl CapabilityPack for FullPack {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &TEST_DESCRIPTOR
        }

        fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
            self.called.store(true, Ordering::SeqCst);
            registrar.register_stage(StageRegistration::new(
                "knowledge",
                "knowledge.stage",
                Arc::new(FullStageHandler),
            ))?;
            registrar.register_ingester(IngesterRegistration::new(
                "knowledge",
                "knowledge.ingester",
                Arc::new(FullIngesterHandler),
            ))?;
            registrar.register_schema_module(SchemaModule {
                capability_id: "knowledge",
                name: "knowledge.schema",
                description: "schema",
            })?;
            static EXAMPLES: [QueryExample; 1] = [QueryExample {
                capability_id: "knowledge",
                name: "knowledge.example",
                query: "knowledge()",
                description: "example",
            }];
            registrar.register_query_examples(&EXAMPLES)?;
            Ok(())
        }

        fn migrations(&self) -> &'static [CapabilityMigration] {
            static MIGRATIONS: [CapabilityMigration; 1] = [CapabilityMigration {
                capability_id: "knowledge",
                version: "1",
                description: "write migration log",
                run: record_migration,
            }];
            &MIGRATIONS
        }

        fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
            static HEALTH_CHECKS: [CapabilityHealthCheck; 2] = [
                CapabilityHealthCheck {
                    name: "healthy",
                    run: healthy_check,
                },
                CapabilityHealthCheck {
                    name: "unhealthy",
                    run: unhealthy_check,
                },
            ];
            &HEALTH_CHECKS
        }
    }

    struct StageOnlyPack;

    impl CapabilityPack for StageOnlyPack {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &TEST_DESCRIPTOR
        }

        fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
            registrar.register_stage(StageRegistration::new(
                "knowledge",
                "knowledge.stage",
                Arc::new(StageOnlyHandler),
            ))
        }
    }

    struct IngesterOnlyPack;

    impl CapabilityPack for IngesterOnlyPack {
        fn descriptor(&self) -> &'static CapabilityDescriptor {
            &TEST_DESCRIPTOR
        }

        fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
            registrar.register_ingester(IngesterRegistration::new(
                "knowledge",
                "knowledge.ingester",
                Arc::new(IngesterOnlyHandler),
            ))
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

    fn make_repo_identity() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops-cli".to_string(),
            identity: "local/bitloops/bitloops-cli".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    fn make_host() -> (TempDir, DevqlCapabilityHost) {
        let temp = TempDir::new().expect("temp dir");
        let repo_root = prepare_repo_root(&temp);
        let host = DevqlCapabilityHost::new(repo_root, make_repo_identity()).expect("host");
        (temp, host)
    }

    fn record_migration(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
        let log_path = ctx.repo_root().join("migrations.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        writeln!(file, "migrated")?;
        Ok(())
    }

    fn healthy_check(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
        let view = ctx.config_view("knowledge").expect("config view");
        assert_eq!(view.capability_id(), "knowledge");
        assert!(ctx.stores().check_relational().is_ok());
        CapabilityHealthResult::ok("healthy")
    }

    fn unhealthy_check(_: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
        CapabilityHealthResult::failed("unhealthy", "details")
    }

    #[test]
    fn register_pack_and_invoke_registered_handlers() {
        let called = Arc::new(AtomicBool::new(false));
        let pack = FullPack::new(Arc::clone(&called));
        let (_temp, mut host) = make_host();

        host.register_pack(&pack).expect("register pack");

        assert!(called.load(Ordering::SeqCst));
        assert!(host.descriptor("knowledge").is_some());
        assert_eq!(host.descriptors().count(), 1);
        assert!(host.has_stage("knowledge", "knowledge.stage"));
        assert_eq!(host.schema_modules().len(), 1);
        assert_eq!(host.query_examples().len(), 1);

        let stage = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                host.invoke_stage("knowledge", "knowledge.stage", json!({ "limit": 4 }))
                    .await
            })
            .expect("invoke stage");
        assert_eq!(stage.payload["limit"], json!(4));
        assert!(stage.payload["repo_root"].is_string());
        assert!(stage.render_human().contains("\"limit\": 4"));

        let ingest = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                host.invoke_ingester(
                    "knowledge",
                    "knowledge.ingester",
                    json!({ "value": "alpha" }),
                )
                .await
            })
            .expect("invoke ingester");
        assert_eq!(ingest.payload["value"], json!("alpha"));
        assert!(ingest.render_human().contains("ingested alpha"));

        let log_path = host.repo_root().join("migrations.log");
        let log = fs::read_to_string(log_path).expect("migration log");
        assert_eq!(log, "migrated\n");
    }

    #[test]
    fn register_pack_rejects_duplicate_stage() {
        let (_temp, mut host) = make_host();

        host.register_pack(&StageOnlyPack)
            .expect("register stage pack");
        let err = host
            .register_pack(&StageOnlyPack)
            .expect_err("duplicate stage should fail");

        assert!(err.to_string().contains("stage `knowledge.stage` is already registered"));
    }

    #[test]
    fn register_pack_rejects_duplicate_ingester() {
        let (_temp, mut host) = make_host();

        host.register_pack(&IngesterOnlyPack)
            .expect("register ingester pack");
        let err = host
            .register_pack(&IngesterOnlyPack)
            .expect_err("duplicate ingester should fail");

        assert!(
            err.to_string()
                .contains("ingester `knowledge.ingester` is already registered")
        );
    }

    #[test]
    fn run_health_checks_returns_prefixed_results() {
        let called = Arc::new(AtomicBool::new(false));
        let pack = FullPack::new(called);
        let (_temp, mut host) = make_host();

        host.register_pack(&pack).expect("register pack");
        let results = host.run_health_checks("knowledge");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "knowledge.healthy");
        assert!(results[0].1.healthy);
        assert_eq!(results[1].0, "knowledge.unhealthy");
        assert!(!results[1].1.healthy);
        assert_eq!(results[1].1.details.as_deref(), Some("details"));
    }
}
