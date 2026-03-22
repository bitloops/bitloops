use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::capability_packs as capabilities;
use crate::engine::devql::RelationalStorage;
use crate::engine::devql::RepoIdentity;

use super::config_view::CapabilityConfigView;
use super::descriptor::CapabilityDescriptor;
use super::health::{CapabilityHealthCheck, CapabilityHealthResult};
use super::lifecycle;
use super::migrations::CapabilityMigration;
use super::policy::{CrossPackAccessPolicy, HostInvocationPolicy, with_timeout};
use super::registrar::{
    CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration, KnowledgeIngester, KnowledgeIngesterRegistration, KnowledgeStage,
    KnowledgeStageRegistration, QueryExample, SchemaModule, StageHandler, StageRegistration,
    StageRequest, StageResponse,
};
use super::runtime_contexts::LocalCapabilityRuntimeResources;

#[derive(Clone)]
enum RegisteredStage {
    Core(Arc<dyn StageHandler>),
    Knowledge(Arc<dyn KnowledgeStage>),
}

#[derive(Clone)]
enum RegisteredIngester {
    Core(Arc<dyn IngesterHandler>),
    Knowledge(Arc<dyn KnowledgeIngester>),
}

pub struct DevqlCapabilityHost {
    runtime: LocalCapabilityRuntimeResources,
    descriptors: HashMap<String, &'static CapabilityDescriptor>,
    stages: HashMap<(String, String), RegisteredStage>,
    ingesters: HashMap<(String, String), RegisteredIngester>,
    schema_modules: Vec<SchemaModule>,
    query_examples: Vec<&'static [QueryExample]>,
    migrations: Vec<CapabilityMigration>,
    health_checks: HashMap<String, Vec<CapabilityHealthCheck>>,
    migrations_applied: bool,
    invocation_policy: HostInvocationPolicy,
    cross_pack_access: CrossPackAccessPolicy,
}

impl DevqlCapabilityHost {
    pub fn new(repo_root: PathBuf, repo: RepoIdentity) -> Result<Self> {
        let runtime = LocalCapabilityRuntimeResources::new(&repo_root, repo)?;
        let invocation_policy = HostInvocationPolicy::from_config_root(&runtime.config_root);
        let cross_pack_access = CrossPackAccessPolicy::from_config_root(&runtime.config_root);
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
            invocation_policy,
            cross_pack_access,
        })
    }

    pub fn builtin(repo_root: impl Into<PathBuf>, repo: RepoIdentity) -> Result<Self> {
        let mut host = Self::new(repo_root.into(), repo)?;
        let packs = capabilities::builtin_packs(host.repo_root())?;
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

    pub fn invocation_policy(&self) -> &HostInvocationPolicy {
        &self.invocation_policy
    }

    pub fn cross_pack_access(&self) -> &CrossPackAccessPolicy {
        &self.cross_pack_access
    }

    pub fn register_pack(&mut self, pack: &dyn CapabilityPack) -> Result<()> {
        lifecycle::register_pack(self, pack)?;
        let descriptor = pack.descriptor();
        self.descriptors
            .insert(descriptor.id.to_string(), descriptor);
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

    /// Snapshot of registered packs, migrations, invocation policy, and cross-pack grants.
    /// Health results are empty; use [`super::diagnostics::collect_health_outcomes`] when needed.
    pub fn registry_report(&self) -> super::diagnostics::HostRegistryReport {
        use super::diagnostics::{
            CrossPackGrantSummary, HostRegistryReport, InvocationSummary, MigrationStepSummary,
            PackRegistryEntry,
        };

        let repo = self.repo();
        let migration_plan: Vec<MigrationStepSummary> = self
            .migrations
            .iter()
            .map(|m| MigrationStepSummary {
                capability_id: m.capability_id.to_string(),
                version: m.version.to_string(),
                description: m.description.to_string(),
            })
            .collect();

        let cross_pack_grants: Vec<CrossPackGrantSummary> = self
            .cross_pack_access()
            .grants
            .iter()
            .map(CrossPackGrantSummary::from)
            .collect();

        let mut pack_ids: Vec<String> = self.descriptors.keys().cloned().collect();
        pack_ids.sort();

        let packs: Vec<PackRegistryEntry> = pack_ids
            .into_iter()
            .filter_map(|id| {
                self.descriptor(&id)
                    .map(|descriptor| self.pack_registry_entry(descriptor))
            })
            .collect();

        HostRegistryReport {
            repo_id: repo.repo_id.clone(),
            repo_identity: repo.identity.clone(),
            repo_root: self.repo_root().display().to_string(),
            migrations_applied_this_session: self.migrations_applied,
            invocation: InvocationSummary::from(self.invocation_policy()),
            cross_pack_grants,
            migration_plan,
            packs,
            health: Vec::new(),
        }
    }

    fn pack_registry_entry(
        &self,
        d: &'static CapabilityDescriptor,
    ) -> super::diagnostics::PackRegistryEntry {
        use super::diagnostics::{MigrationStepSummary, PackRegistryEntry, SchemaModuleSummary};

        let id = d.id.to_string();

        let mut stages: Vec<String> = self
            .stages
            .keys()
            .filter(|(cap, _)| cap == &id)
            .map(|(_, name)| name.clone())
            .collect();
        stages.sort();

        let mut ingesters: Vec<String> = self
            .ingesters
            .keys()
            .filter(|(cap, _)| cap == &id)
            .map(|(_, name)| name.clone())
            .collect();
        ingesters.sort();

        let migrations: Vec<MigrationStepSummary> = self
            .migrations
            .iter()
            .filter(|m| m.capability_id == d.id)
            .map(|m| MigrationStepSummary {
                capability_id: m.capability_id.to_string(),
                version: m.version.to_string(),
                description: m.description.to_string(),
            })
            .collect();

        let schema_modules: Vec<SchemaModuleSummary> = self
            .schema_modules
            .iter()
            .filter(|m| m.capability_id == d.id)
            .map(|m| SchemaModuleSummary {
                name: m.name.to_string(),
                description: m.description.to_string(),
            })
            .collect();

        let mut health_check_names: Vec<String> = self
            .health_checks
            .get(&id)
            .map(|checks| checks.iter().map(|c| c.name.to_string()).collect())
            .unwrap_or_default();
        health_check_names.sort();

        let query_example_count: usize = self
            .query_examples
            .iter()
            .map(|chunk| chunk.iter().filter(|ex| ex.capability_id == d.id).count())
            .sum();

        let dependencies: Vec<String> = d
            .dependencies
            .iter()
            .map(|dep| format!("{} (>={})", dep.capability_id, dep.min_version))
            .collect();

        PackRegistryEntry {
            id,
            display_name: d.display_name.to_string(),
            version: d.version.to_string(),
            api_version: d.api_version,
            default_enabled: d.default_enabled,
            experimental: d.experimental,
            dependencies,
            stages,
            ingesters,
            migrations,
            schema_modules,
            health_check_names,
            query_example_count,
        }
    }

    pub async fn invoke_ingester(
        &mut self,
        capability_id: &str,
        ingester_name: &str,
        payload: Value,
    ) -> Result<IngestResult> {
        self.invoke_ingester_with_relational(capability_id, ingester_name, payload, None)
            .await
    }

    pub async fn invoke_ingester_with_relational(
        &mut self,
        capability_id: &str,
        ingester_name: &str,
        payload: Value,
        devql_relational: Option<&RelationalStorage>,
    ) -> Result<IngestResult> {
        self.ensure_migrations_applied()?;

        let key = (capability_id.to_string(), ingester_name.to_string());
        let handler = self.ingesters.get(&key).cloned();
        let Some(handler) = handler else {
            bail!(
                "[capability_pack:{capability_id}] [ingester:{ingester_name}] not registered on DevqlCapabilityHost"
            );
        };

        let request = IngestRequest::new(payload);
        let mut runtime = self.runtime.runtime_with_relational(
            devql_relational,
            Some(capability_id),
            Some(ingester_name),
        );
        let limit = self.invocation_policy.ingester_timeout;
        match handler {
            RegisteredIngester::Core(h) => {
                with_timeout(
                    "capability ingester",
                    limit,
                    h.ingest(request, &mut runtime),
                )
                .await
            }
            RegisteredIngester::Knowledge(h) => {
                with_timeout(
                    "capability ingester",
                    limit,
                    h.ingest(request, &mut runtime),
                )
                .await
            }
        }
    }

    pub async fn invoke_stage(
        &mut self,
        capability_id: &str,
        stage_name: &str,
        payload: Value,
    ) -> Result<StageResponse> {
        self.ensure_migrations_applied()?;

        let key = (capability_id.to_string(), stage_name.to_string());
        let handler = self.stages.get(&key).cloned();
        let Some(handler) = handler else {
            bail!(
                "[capability_pack:{capability_id}] [stage:{stage_name}] not registered on DevqlCapabilityHost"
            );
        };

        let request = StageRequest::new(payload);
        let mut runtime = self.runtime.runtime();
        let limit = self.invocation_policy.stage_timeout;
        match handler {
            RegisteredStage::Core(h) => {
                with_timeout("capability stage", limit, h.execute(request, &mut runtime)).await
            }
            RegisteredStage::Knowledge(h) => {
                with_timeout("capability stage", limit, h.execute(request, &mut runtime)).await
            }
        }
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
        self.ensure_migrations_applied_sync()
    }

    /// Run registered pack migrations synchronously (e.g. during `devql init` before async ingest).
    pub fn ensure_migrations_applied_sync(&mut self) -> Result<()> {
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
        let key = (
            stage.capability_id.to_string(),
            stage.stage_name.to_string(),
        );
        if self.stages.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [stage:{}] duplicate registration",
                stage.capability_id,
                stage.stage_name
            );
        }
        self.stages
            .insert(key, RegisteredStage::Core(stage.handler));
        Ok(())
    }

    fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()> {
        let key = (
            ingester.capability_id.to_string(),
            ingester.ingester_name.to_string(),
        );
        if self.ingesters.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [ingester:{}] duplicate registration",
                ingester.capability_id,
                ingester.ingester_name
            );
        }
        self.ingesters
            .insert(key, RegisteredIngester::Core(ingester.handler));
        Ok(())
    }

    fn register_knowledge_stage(&mut self, stage: KnowledgeStageRegistration) -> Result<()> {
        let key = (
            stage.capability_id.to_string(),
            stage.stage_name.to_string(),
        );
        if self.stages.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [knowledge_stage:{}] duplicate registration",
                stage.capability_id,
                stage.stage_name
            );
        }
        self.stages
            .insert(key, RegisteredStage::Knowledge(stage.handler));
        Ok(())
    }

    fn register_knowledge_ingester(
        &mut self,
        ingester: KnowledgeIngesterRegistration,
    ) -> Result<()> {
        let key = (
            ingester.capability_id.to_string(),
            ingester.ingester_name.to_string(),
        );
        if self.ingesters.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [knowledge_ingester:{}] duplicate registration",
                ingester.capability_id,
                ingester.ingester_name
            );
        }
        self.ingesters
            .insert(key, RegisteredIngester::Knowledge(ingester.handler));
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
    use crate::engine::devql::capability_host::migrations::{CapabilityMigration, MigrationRunner};
    use crate::engine::devql::capability_host::registrar::{
        BoxFuture, CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult,
        IngesterHandler, IngesterRegistration, QueryExample, SchemaModule, StageHandler,
        StageRegistration, StageRequest, StageResponse,
    };
    use crate::utils::paths;
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
                run: MigrationRunner::Core(record_migration),
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

        let msg = err.to_string();
        assert!(
            msg.contains("[stage:knowledge.stage]") && msg.contains("duplicate registration"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn register_pack_rejects_duplicate_ingester() {
        let (_temp, mut host) = make_host();

        host.register_pack(&IngesterOnlyPack)
            .expect("register ingester pack");
        let err = host
            .register_pack(&IngesterOnlyPack)
            .expect_err("duplicate ingester should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("[ingester:knowledge.ingester]") && msg.contains("duplicate registration"),
            "unexpected error: {msg}"
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
