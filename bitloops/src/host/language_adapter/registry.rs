use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::host::extension_host::{
    CapabilityMigrationStatus, CapabilityMigrationStep, ExtensionLifecycleState,
    ExtensionReadinessFailure, ExtensionReadinessReport, ExtensionReadinessStatus,
    orchestrate_capability_migrations,
};

use super::{
    DependencyEdge, LanguageAdapterContext, LanguageAdapterError, LanguageAdapterHealthCheck,
    LanguageAdapterHealthContext, LanguageAdapterHealthResult, LanguageAdapterMigrationContext,
    LanguageAdapterMigrationDescriptor, LanguageAdapterMigrationExecution,
    LanguageAdapterMigrationFailure, LanguageAdapterMigrationRunReport,
    LanguageAdapterMigrationStatus, LanguageAdapterMigrationStep, LanguageAdapterPack,
    LanguageArtefact, LanguageTestSupport,
};

#[derive(Debug, Default)]
struct LanguageAdapterLifecycleState {
    migrated_pack_ids: HashSet<String>,
    applied_migrations: Vec<LanguageAdapterMigrationExecution>,
}

pub(crate) struct LanguageAdapterRegistry {
    packs: HashMap<String, Arc<dyn LanguageAdapterPack>>,
    migrations: HashMap<String, &'static [LanguageAdapterMigrationDescriptor]>,
    health_checks: HashMap<String, &'static [LanguageAdapterHealthCheck]>,
    lifecycle: RwLock<LanguageAdapterLifecycleState>,
}

impl LanguageAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self {
            packs: HashMap::new(),
            migrations: HashMap::new(),
            health_checks: HashMap::new(),
            lifecycle: RwLock::new(LanguageAdapterLifecycleState::default()),
        }
    }

    pub(crate) fn register(
        &mut self,
        pack: Box<dyn LanguageAdapterPack>,
    ) -> Result<(), LanguageAdapterError> {
        let descriptor = pack.descriptor();
        let pack_id = descriptor.id.to_string();

        let supported = pack.supported_language_kinds();
        for mapping in pack.canonical_mappings() {
            if !supported.contains(&mapping.language_kind) {
                return Err(LanguageAdapterError::InvalidCanonicalMapping {
                    pack_id: pack_id.clone(),
                    language_kind: mapping.language_kind.to_string(),
                    reason: "language_kind not in supported_language_kinds".to_string(),
                });
            }
        }

        self.migrations.insert(pack_id.clone(), pack.migrations());
        self.health_checks
            .insert(pack_id.clone(), pack.health_checks());
        self.packs.insert(pack_id, Arc::from(pack));
        Ok(())
    }

    pub(crate) fn with_builtins(
        packs: Vec<Box<dyn LanguageAdapterPack>>,
    ) -> Result<Self, LanguageAdapterError> {
        let mut registry = Self::new();
        for pack in packs {
            registry.register(pack)?;
        }
        Ok(registry)
    }

    pub(crate) fn get(&self, pack_id: &str) -> Option<Arc<dyn LanguageAdapterPack>> {
        self.packs.get(pack_id).cloned()
    }

    pub(crate) fn extract_artefacts(
        &self,
        pack_id: &str,
        content: &str,
        path: &str,
    ) -> Result<Vec<LanguageArtefact>> {
        let pack = self
            .packs
            .get(pack_id)
            .ok_or_else(|| anyhow::anyhow!("language adapter pack `{pack_id}` not found"))?;
        pack.extract_artefacts(content, path)
    }

    pub(crate) fn extract_dependency_edges(
        &self,
        pack_id: &str,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        let pack = self
            .packs
            .get(pack_id)
            .ok_or_else(|| anyhow::anyhow!("language adapter pack `{pack_id}` not found"))?;
        pack.extract_dependency_edges(content, path, artefacts)
    }

    pub(crate) fn extract_file_docstring(&self, pack_id: &str, content: &str) -> Option<String> {
        self.packs.get(pack_id)?.extract_file_docstring(content)
    }

    pub(crate) fn registered_pack_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.packs.keys().map(String::as_str).collect();
        ids.sort();
        ids
    }

    pub(crate) fn test_support_for_pack(
        &self,
        pack_id: &str,
    ) -> Option<Arc<dyn LanguageTestSupport>> {
        self.packs.get(pack_id)?.test_support()
    }

    pub(crate) fn all_test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
        let mut supports = self
            .registered_pack_ids()
            .into_iter()
            .filter_map(|pack_id| self.test_support_for_pack(pack_id))
            .collect::<Vec<_>>();
        supports.sort_by_key(|support| support.priority());
        supports
    }

    pub(crate) fn migration_count_for(&self, pack_id: &str) -> usize {
        self.migrations.get(pack_id).map_or(0, |steps| steps.len())
    }

    pub(crate) fn health_check_names_for(&self, pack_id: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .health_checks
            .get(pack_id)
            .map(|checks| checks.iter().map(|check| check.name.to_string()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    pub(crate) fn migration_plan(&self) -> Vec<LanguageAdapterMigrationStep> {
        let mut steps = Vec::new();
        for pack_id in self.registered_pack_ids() {
            let Some(descriptors) = self.migrations.get(pack_id) else {
                continue;
            };
            for descriptor in *descriptors {
                steps.push(LanguageAdapterMigrationStep::from_descriptor(
                    pack_id, descriptor,
                ));
            }
        }
        steps.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.pack_id.cmp(&right.pack_id))
                .then_with(|| left.migration_id.cmp(&right.migration_id))
        });
        steps
    }

    pub(crate) fn run_migrations(
        &self,
        adapter_context: &LanguageAdapterContext,
    ) -> LanguageAdapterMigrationRunReport {
        let total_plan = self.migration_plan();
        let applied_keys: HashSet<(String, String)> = {
            let state = self
                .lifecycle
                .read()
                .expect("language adapter lifecycle state read lock");
            state
                .applied_migrations
                .iter()
                .map(|execution| (execution.pack_id.clone(), execution.migration_id.clone()))
                .collect()
        };

        let pending_steps: Vec<LanguageAdapterMigrationStep> = total_plan
            .iter()
            .filter(|step| {
                !applied_keys.contains(&(step.pack_id.clone(), step.migration_id.clone()))
            })
            .cloned()
            .collect();

        let packs_without_migrations: Vec<String> = self
            .registered_pack_ids()
            .into_iter()
            .filter(|pack_id| self.migration_count_for(pack_id) == 0)
            .map(str::to_string)
            .collect();

        let orchestration_steps: Vec<CapabilityMigrationStep> = pending_steps
            .iter()
            .map(|step| CapabilityMigrationStep {
                pack_id: step.pack_id.clone(),
                migration_id: step.migration_id.clone(),
                order: step.order,
                description: step.description.clone(),
            })
            .collect();

        let orchestration = orchestrate_capability_migrations(orchestration_steps, |step| {
            let Some(descriptor) = self.migration_descriptor_for(&step.pack_id, &step.migration_id)
            else {
                return Err(format!(
                    "language adapter migration descriptor not found: pack=`{}` migration=`{}`",
                    step.pack_id, step.migration_id
                ));
            };
            let context = LanguageAdapterMigrationContext::new(
                step.pack_id.clone(),
                step.migration_id.clone(),
                step.order,
                step.description.clone(),
                adapter_context.clone(),
            );
            (descriptor.run)(&context).map_err(|error| error.to_string())
        });

        let report = LanguageAdapterMigrationRunReport {
            status: match orchestration.status {
                CapabilityMigrationStatus::Completed => LanguageAdapterMigrationStatus::Completed,
                CapabilityMigrationStatus::Failed => LanguageAdapterMigrationStatus::Failed,
            },
            executed: orchestration
                .executed
                .iter()
                .map(|execution| LanguageAdapterMigrationExecution {
                    pack_id: execution.pack_id.clone(),
                    migration_id: execution.migration_id.clone(),
                    order: execution.order,
                })
                .collect(),
            failure: orchestration.failure.as_ref().map(|failure| {
                LanguageAdapterMigrationFailure {
                    pack_id: failure.pack_id.clone(),
                    migration_id: failure.migration_id.clone(),
                    order: failure.order,
                    reason: failure.reason.clone(),
                }
            }),
        };

        self.persist_migration_state(&report, &total_plan, &packs_without_migrations);
        report
    }

    pub(crate) fn run_health_checks(
        &self,
        pack_id: &str,
        runtime: &str,
    ) -> Vec<(String, LanguageAdapterHealthResult)> {
        let checks = self.health_checks.get(pack_id).copied().unwrap_or(&[]);
        let context = self.health_context(pack_id, runtime);
        checks
            .iter()
            .map(|check| (format!("{pack_id}.{}", check.name), (check.run)(&context)))
            .collect()
    }

    pub(crate) fn collect_health_outcomes(
        &self,
        runtime: &str,
    ) -> Vec<(String, LanguageAdapterHealthResult)> {
        let mut outcomes = Vec::new();
        for pack_id in self.registered_pack_ids() {
            outcomes.extend(self.run_health_checks(pack_id, runtime));
        }
        outcomes
    }

    pub(crate) fn readiness_reports(
        &self,
        runtime: &str,
        include_health_checks: bool,
    ) -> Vec<ExtensionReadinessReport> {
        let mut reports = Vec::new();
        for pack_id in self.registered_pack_ids() {
            let mut failures = Vec::new();
            let pending_migration_count = self.pending_migration_count_for(pack_id);
            if pending_migration_count > 0 {
                failures.push(ExtensionReadinessFailure {
                    code: "migrations_pending".to_string(),
                    message: format!(
                        "language adapter pack has unapplied migrations in runtime `{runtime}`"
                    ),
                });
            }

            if include_health_checks {
                for (check_id, result) in self.run_health_checks(pack_id, runtime) {
                    if result.is_healthy() {
                        continue;
                    }
                    let check_name = check_id
                        .split_once('.')
                        .map(|(_, name)| name)
                        .unwrap_or(check_id.as_str());
                    let details = result.details.unwrap_or_default();
                    failures.push(ExtensionReadinessFailure {
                        code: format!("health_check_failed_{check_name}"),
                        message: if details.is_empty() {
                            format!("language adapter health check `{check_name}` failed")
                        } else {
                            format!(
                                "language adapter health check `{check_name}` failed: {details}"
                            )
                        },
                    });
                }
            }

            let status = ExtensionReadinessStatus::from_failures(!failures.is_empty());
            let lifecycle_state = if status == ExtensionReadinessStatus::Ready {
                if self.migration_count_for(pack_id) > 0 {
                    ExtensionLifecycleState::Migrated
                } else {
                    ExtensionLifecycleState::Ready
                }
            } else {
                ExtensionLifecycleState::Registered
            };

            reports.push(ExtensionReadinessReport {
                family: "language-adapter-pack".to_string(),
                id: pack_id.to_string(),
                registered: true,
                ready: status == ExtensionReadinessStatus::Ready,
                status,
                lifecycle_state,
                failures,
            });
        }
        reports
    }

    pub(crate) fn migrated_pack_ids(&self) -> Vec<String> {
        let state = self
            .lifecycle
            .read()
            .expect("language adapter lifecycle state read lock");
        let mut ids: Vec<String> = state.migrated_pack_ids.iter().cloned().collect();
        ids.sort();
        ids
    }

    pub(crate) fn applied_migrations(&self) -> Vec<LanguageAdapterMigrationExecution> {
        let state = self
            .lifecycle
            .read()
            .expect("language adapter lifecycle state read lock");
        let mut applied = state.applied_migrations.clone();
        applied.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.pack_id.cmp(&right.pack_id))
                .then_with(|| left.migration_id.cmp(&right.migration_id))
        });
        applied
    }

    fn migration_descriptor_for(
        &self,
        pack_id: &str,
        migration_id: &str,
    ) -> Option<&'static LanguageAdapterMigrationDescriptor> {
        let descriptors = self.migrations.get(pack_id)?;
        descriptors
            .iter()
            .find(|descriptor| descriptor.id == migration_id)
    }

    fn pending_migration_count_for(&self, pack_id: &str) -> usize {
        let total = self.migration_count_for(pack_id);
        if total == 0 {
            return 0;
        }
        let state = self
            .lifecycle
            .read()
            .expect("language adapter lifecycle state read lock");
        let applied = state
            .applied_migrations
            .iter()
            .filter(|execution| execution.pack_id == pack_id)
            .count();
        total.saturating_sub(applied)
    }

    fn health_context(&self, pack_id: &str, runtime: &str) -> LanguageAdapterHealthContext {
        LanguageAdapterHealthContext::new(
            pack_id,
            runtime,
            self.packs.contains_key(pack_id),
            self.migrated_pack_ids().iter().any(|id| id == pack_id),
            self.pending_migration_count_for(pack_id),
        )
    }

    fn persist_migration_state(
        &self,
        report: &LanguageAdapterMigrationRunReport,
        total_plan: &[LanguageAdapterMigrationStep],
        packs_without_migrations: &[String],
    ) {
        let mut total_steps_per_pack: HashMap<String, usize> = HashMap::new();
        for step in total_plan {
            *total_steps_per_pack
                .entry(step.pack_id.clone())
                .or_insert(0) += 1;
        }

        let mut state = self
            .lifecycle
            .write()
            .expect("language adapter lifecycle state write lock");

        for pack_id in packs_without_migrations {
            state.migrated_pack_ids.insert(pack_id.clone());
        }

        for execution in &report.executed {
            if state.applied_migrations.iter().any(|existing| {
                existing.pack_id == execution.pack_id
                    && existing.migration_id == execution.migration_id
            }) {
                continue;
            }
            state.applied_migrations.push(execution.clone());
        }

        let mut applied_steps_per_pack: HashMap<String, usize> = HashMap::new();
        for execution in &state.applied_migrations {
            *applied_steps_per_pack
                .entry(execution.pack_id.clone())
                .or_insert(0) += 1;
        }

        for (pack_id, total_steps) in total_steps_per_pack {
            let applied = applied_steps_per_pack
                .get(&pack_id)
                .copied()
                .unwrap_or_default();
            if applied >= total_steps {
                state.migrated_pack_ids.insert(pack_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    use crate::host::extension_host::LanguagePackDescriptor;
    use crate::host::language_adapter::{
        CanonicalMapping, LanguageKind, MappingCondition, RustKind,
    };

    use super::*;

    static MIGRATION_EXECUTED: AtomicBool = AtomicBool::new(false);

    const TEST_DESCRIPTOR: LanguagePackDescriptor =
        LanguagePackDescriptor {
            id: "test-language-pack",
            version: "1.0.0",
            api_version: 1,
            display_name: "Test Language Pack",
            aliases: &["test-pack"],
            supported_languages: &["testlang"],
            language_profiles: &[],
            compatibility: crate::host::extension_host::ExtensionCompatibility::phase1_local_cli(
                &["language-packs", "readiness", "diagnostics"],
            ),
        };

    fn migration_mark_executed(_ctx: &LanguageAdapterMigrationContext) -> Result<()> {
        MIGRATION_EXECUTED.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn health_fails_when_pending(
        ctx: &LanguageAdapterHealthContext,
    ) -> LanguageAdapterHealthResult {
        if ctx.has_pending_migrations() {
            LanguageAdapterHealthResult::failed("pending migrations", "apply migrations first")
        } else {
            LanguageAdapterHealthResult::ok("healthy")
        }
    }

    static TEST_MIGRATIONS: &[LanguageAdapterMigrationDescriptor] =
        &[LanguageAdapterMigrationDescriptor {
            id: "001",
            order: 1,
            description: "apply test migration",
            run: migration_mark_executed,
        }];

    static TEST_HEALTH_CHECKS: &[LanguageAdapterHealthCheck] = &[LanguageAdapterHealthCheck {
        name: "pending-migrations",
        run: health_fails_when_pending,
    }];
    static TEST_CANONICAL_MAPPINGS: &[CanonicalMapping] = &[CanonicalMapping {
        language_kind: LanguageKind::rust(RustKind::FunctionItem),
        projection: crate::host::devql::CanonicalKindProjection::Function,
        condition: MappingCondition::Always,
    }];
    static TEST_SUPPORTED_LANGUAGE_KINDS: &[LanguageKind] =
        &[LanguageKind::rust(RustKind::FunctionItem)];

    struct TestPack;

    impl LanguageAdapterPack for TestPack {
        fn descriptor(&self) -> &'static LanguagePackDescriptor {
            &TEST_DESCRIPTOR
        }

        fn canonical_mappings(&self) -> &'static [CanonicalMapping] {
            TEST_CANONICAL_MAPPINGS
        }

        fn supported_language_kinds(&self) -> &'static [LanguageKind] {
            TEST_SUPPORTED_LANGUAGE_KINDS
        }

        fn extract_artefacts(&self, _content: &str, _path: &str) -> Result<Vec<LanguageArtefact>> {
            Ok(Vec::new())
        }

        fn extract_dependency_edges(
            &self,
            _content: &str,
            _path: &str,
            _artefacts: &[LanguageArtefact],
        ) -> Result<Vec<DependencyEdge>> {
            Ok(Vec::new())
        }

        fn migrations(&self) -> &'static [LanguageAdapterMigrationDescriptor] {
            TEST_MIGRATIONS
        }

        fn health_checks(&self) -> &'static [LanguageAdapterHealthCheck] {
            TEST_HEALTH_CHECKS
        }
    }

    #[test]
    fn registry_executes_migrations_and_updates_readiness() {
        MIGRATION_EXECUTED.store(false, Ordering::SeqCst);

        let registry =
            LanguageAdapterRegistry::with_builtins(vec![Box::new(TestPack)]).expect("registry");
        let before = registry.readiness_reports("local-cli", true);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].status, ExtensionReadinessStatus::NotReady);
        assert!(
            before[0]
                .failures
                .iter()
                .any(|failure| failure.code == "migrations_pending")
        );

        let context = LanguageAdapterContext::new(
            PathBuf::from("/tmp/repo"),
            "repo-id",
            Some("abc123".to_string()),
        );
        let report = registry.run_migrations(&context);
        assert_eq!(report.status, LanguageAdapterMigrationStatus::Completed);
        assert!(
            MIGRATION_EXECUTED.load(Ordering::SeqCst),
            "migration runner should be invoked"
        );
        assert_eq!(registry.migrated_pack_ids(), vec!["test-language-pack"]);

        let after = registry.readiness_reports("local-cli", true);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].status, ExtensionReadinessStatus::Ready);
    }
}
