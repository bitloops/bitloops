use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::capability_packs as capabilities;
use crate::host::devql::RepoIdentity;
use crate::host::relational_store::DefaultRelationalStore;

use super::config_view::CapabilityConfigView;
use super::descriptor::CapabilityDescriptor;
use super::events::{CurrentStateConsumerContext, HostEventHandler};
use super::gateways::{
    CapabilityWorkplaneGateway, DefaultHostServicesGateway, HostServicesGateway,
    LanguageServicesGateway, SqliteRelationalGateway,
};
use super::health::CapabilityHealthCheck;
use super::lifecycle;
use super::migrations::CapabilityMigration;
use super::policy::{CrossPackAccessPolicy, HostInvocationPolicy};
use super::registrar::{
    CapabilityMailboxRegistration, CapabilityPack, CapabilityRegistrar,
    CurrentStateConsumerRegistration, IngesterHandler, KnowledgeIngesterHandler,
    KnowledgeStageHandler, QueryExample, SchemaModule, StageHandler,
};
use super::runtime_contexts::LocalCapabilityRuntimeResources;
use crate::host::inference::{InferenceGateway, ScopedInferenceGateway};

#[path = "host/execution.rs"]
mod execution;
#[path = "host/registrar.rs"]
mod registrar;
#[cfg(test)]
mod tests;

#[derive(Clone)]
enum RegisteredStage {
    Core(Arc<dyn StageHandler>),
    Knowledge(Arc<dyn KnowledgeStageHandler>),
}

#[derive(Clone)]
enum RegisteredIngester {
    Core(Arc<dyn IngesterHandler>),
    Knowledge(Arc<dyn KnowledgeIngesterHandler>),
}

struct RuntimeLanguageServicesGateway {
    inner: &'static crate::host::capability_host::runtime_contexts::BuiltinLanguageServicesGateway,
}

impl LanguageServicesGateway for RuntimeLanguageServicesGateway {
    fn test_supports(&self) -> Vec<Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        self.inner.test_supports()
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        self.inner.resolve_test_support_for_path(relative_path)
    }
}

pub struct DevqlCapabilityHost {
    runtime: LocalCapabilityRuntimeResources,
    descriptors: HashMap<String, &'static CapabilityDescriptor>,
    stages: HashMap<(String, String), RegisteredStage>,
    ingesters: HashMap<(String, String), RegisteredIngester>,
    current_state_consumers: Vec<CurrentStateConsumerRegistration>,
    mailboxes: Vec<CapabilityMailboxRegistration>,
    event_handlers: Vec<Arc<dyn HostEventHandler>>,
    schema_modules: Vec<SchemaModule>,
    query_examples: Vec<&'static [QueryExample]>,
    migrations: Vec<CapabilityMigration>,
    health_checks: HashMap<String, Vec<CapabilityHealthCheck>>,
    migrations_applied: AtomicBool,
    migration_lock: Mutex<()>,
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
            current_state_consumers: Vec::new(),
            mailboxes: Vec::new(),
            event_handlers: Vec::new(),
            schema_modules: Vec::new(),
            query_examples: Vec::new(),
            migrations: Vec::new(),
            health_checks: HashMap::new(),
            migrations_applied: AtomicBool::new(false),
            migration_lock: Mutex::new(()),
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

    pub fn inference(&self) -> &dyn InferenceGateway {
        &self.runtime.inference
    }

    pub fn inference_for_capability<'a>(
        &'a self,
        capability_id: &'a str,
    ) -> ScopedInferenceGateway<'a> {
        self.runtime.inference.scoped(Some(capability_id))
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

    pub fn event_handlers(&self) -> &[Arc<dyn HostEventHandler>] {
        self.event_handlers.as_slice()
    }

    pub fn current_state_consumers(&self) -> &[CurrentStateConsumerRegistration] {
        self.current_state_consumers.as_slice()
    }

    pub fn workplane_mailboxes(&self) -> &[CapabilityMailboxRegistration] {
        self.mailboxes.as_slice()
    }

    pub fn declared_mailboxes_for_capability(
        &self,
        capability_id: &str,
    ) -> Vec<CapabilityMailboxRegistration> {
        self.mailboxes
            .iter()
            .copied()
            .filter(|registration| registration.capability_id == capability_id)
            .collect()
    }

    pub fn mailbox_registration(
        &self,
        capability_id: &str,
        mailbox_name: &str,
    ) -> Option<CapabilityMailboxRegistration> {
        self.mailboxes.iter().copied().find(|registration| {
            registration.capability_id == capability_id && registration.mailbox_name == mailbox_name
        })
    }

    pub fn build_workplane_gateway(
        &self,
        capability_id: &str,
    ) -> Result<Arc<dyn CapabilityWorkplaneGateway>> {
        Ok(Arc::new(self.runtime.workplane_gateway_for_capability(
            capability_id,
            &self.declared_mailboxes_for_capability(capability_id),
        )?))
    }

    pub fn build_current_state_consumer_context(
        &self,
        capability_id: &str,
    ) -> Result<CurrentStateConsumerContext> {
        let relational_store = DefaultRelationalStore::open_local_for_backend_config(
            self.repo_root(),
            &self.runtime.backends.relational,
        )?;
        let sqlite_pool = relational_store.local_sqlite_pool_allow_create()?;

        let language_services: Arc<dyn LanguageServicesGateway> =
            Arc::new(RuntimeLanguageServicesGateway {
                inner: self.runtime.languages,
            });
        let relational = Arc::new(SqliteRelationalGateway::new(sqlite_pool));
        let host_services: Arc<dyn HostServicesGateway> = Arc::new(
            DefaultHostServicesGateway::new(self.runtime.repo.repo_id.clone()),
        );
        let workplane = Arc::new(self.runtime.workplane_gateway_for_capability(
            capability_id,
            &self.declared_mailboxes_for_capability(capability_id),
        )?);

        Ok(CurrentStateConsumerContext {
            config_root: self.runtime.config_root.clone(),
            storage: Arc::new(relational_store.to_local_inner()),
            relational,
            language_services,
            host_services,
            workplane,
        })
    }

    pub fn build_event_handler_context(&self) -> Result<CurrentStateConsumerContext> {
        self.build_current_state_consumer_context("<event_handler>")
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
            migrations_applied_this_session: self.migrations_applied.load(Ordering::Acquire),
            invocation: InvocationSummary::from(self.invocation_policy()),
            cross_pack_grants,
            migration_plan,
            packs,
            language_adapters: super::diagnostics::LanguageAdapterLifecycleSummary::default(),
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
}
