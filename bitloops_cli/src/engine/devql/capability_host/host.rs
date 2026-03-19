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
