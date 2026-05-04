use super::*;
use crate::host::capability_host::contexts::{
    CapabilityExecutionContext, CapabilityHealthContext, CapabilityIngestContext,
    CapabilityMigrationContext,
};
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};
use crate::host::capability_host::migrations::{CapabilityMigration, MigrationRunner};
use crate::host::capability_host::registrar::{
    BoxFuture, CapabilityPack, CapabilityRegistrar, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration, QueryExample, SchemaModule, StageHandler, StageRegistration,
    StageRequest, StageResponse,
};
use crate::host::devql::RepoIdentity;
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

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

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
    inference_slots: &[],
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
    for path in [
        paths::default_relational_db_path(&repo_root),
        paths::default_events_db_path(&repo_root),
    ] {
        fs::create_dir_all(path.parent().expect("default store path has parent"))
            .expect("create store dir");
    }
    fs::create_dir_all(paths::default_blob_store_path(&repo_root)).expect("create blob dir");
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

    let stage = test_runtime()
        .block_on(async {
            host.invoke_stage("knowledge", "knowledge.stage", json!({ "limit": 4 }))
                .await
        })
        .expect("invoke stage");
    assert_eq!(stage.payload["limit"], json!(4));
    assert!(stage.payload["repo_root"].is_string());
    assert!(stage.render_human().contains("\"limit\": 4"));

    let ingest = test_runtime()
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
