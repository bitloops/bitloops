// Inherent impl for `CoreExtensionHost`. Rust forbids `include!` inside `impl` blocks, so this file
// is one unit; subsections below mirror domains.
impl CoreExtensionHost {
    // --- Bootstrap & pack registration ---

    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Result<Self, CoreExtensionHostError> {
        let mut host = Self::new();
        host.bootstrap_builtins()?;
        Ok(host)
    }

    pub fn compatibility_context(&self) -> HostCompatibilityContext {
        self.compatibility_context
    }

    pub fn bootstrap_builtins(&mut self) -> Result<(), CoreExtensionHostError> {
        self.register_language_pack(CSHARP_LANGUAGE_PACK)?;
        self.register_language_pack(RUST_LANGUAGE_PACK)?;
        self.register_language_pack(TS_JS_LANGUAGE_PACK)?;
        self.register_language_pack(PYTHON_LANGUAGE_PACK)?;
        self.register_language_pack(GO_LANGUAGE_PACK)?;
        self.register_language_pack(JAVA_LANGUAGE_PACK)?;
        self.register_language_pack(PHP_LANGUAGE_PACK)?;
        self.register_capability_pack(KNOWLEDGE_CAPABILITY_PACK)?;
        self.register_capability_pack(TEST_HARNESS_CAPABILITY_PACK)?;
        Ok(())
    }

    pub fn register_language_pack(
        &mut self,
        descriptor: LanguagePackDescriptor,
    ) -> Result<(), CoreExtensionHostError> {
        let pack_id = descriptor.id.to_ascii_lowercase();
        if let Err(error) = descriptor.compatibility.validate(
            "language pack",
            descriptor.id,
            self.compatibility_context,
        ) {
            self.push_diagnostic(
                "language-pack",
                descriptor.id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Compatibility,
                "compatibility_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        if let Err(error) = self.language_packs.register(descriptor) {
            self.push_diagnostic(
                "language-pack",
                &pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Registration,
                "registration_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        self.push_diagnostic(
            "language-pack",
            &pack_id,
            ExtensionDiagnosticSeverity::Info,
            ExtensionDiagnosticKind::Registration,
            "registered",
            "language pack registered".to_string(),
        );
        Ok(())
    }

    pub fn register_capability_pack(
        &mut self,
        descriptor: CapabilityPackDescriptor,
    ) -> Result<(), CoreExtensionHostError> {
        let pack_id = descriptor.id().to_ascii_lowercase();
        if let Err(error) = descriptor.compatibility.validate(
            "capability pack",
            descriptor.id(),
            self.compatibility_context,
        ) {
            self.push_diagnostic(
                "capability-pack",
                descriptor.id(),
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Compatibility,
                "compatibility_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        if let Err(error) = self.capability_packs.register(descriptor) {
            self.push_diagnostic(
                "capability-pack",
                &pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Registration,
                "registration_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        self.push_diagnostic(
            "capability-pack",
            &pack_id,
            ExtensionDiagnosticSeverity::Info,
            ExtensionDiagnosticKind::Registration,
            "registered",
            "capability pack registered".to_string(),
        );
        Ok(())
    }

    // --- Capability migrations ---

    pub fn capability_migration_plan(&self) -> Vec<CapabilityMigrationStep> {
        let mut steps = Vec::new();
        for pack_id in self.capability_packs.registered_pack_ids() {
            let Some(descriptor) = self.capability_packs.resolve_pack(pack_id) else {
                continue;
            };
            for migration in descriptor.migrations {
                steps.push(CapabilityMigrationStep::from_descriptor(pack_id, migration));
            }
        }
        steps
    }

    pub fn run_capability_migrations<F>(&mut self, mut executor: F) -> CapabilityMigrationRunReport
    where
        F: FnMut(&CapabilityMigrationContext) -> Result<(), String>,
    {
        let steps = self.capability_migration_plan();
        let mut steps_per_pack: HashMap<String, usize> = HashMap::new();
        let mut packs_without_migrations = Vec::new();

        for step in &steps {
            let entry = steps_per_pack.entry(step.pack_id.clone()).or_insert(0);
            *entry += 1;
        }

        for pack_id in self.capability_packs.registered_pack_ids() {
            let Some(descriptor) = self.capability_packs.resolve_pack(pack_id) else {
                continue;
            };
            if descriptor.migrations.is_empty() {
                packs_without_migrations.push(pack_id.to_string());
            }
        }

        let report = orchestrate_capability_migrations(steps, |step| {
            let context = CapabilityMigrationContext::new(
                step.pack_id.clone(),
                step.migration_id.clone(),
                step.order,
                step.description.clone(),
            );
            executor(&context)
        });

        for pack_id in packs_without_migrations {
            self.migrated_capability_packs.insert(pack_id);
        }

        let mut executed_per_pack: HashMap<String, usize> = HashMap::new();
        for execution in &report.executed {
            self.applied_migrations.push(execution.clone());
            let entry = executed_per_pack
                .entry(execution.pack_id.clone())
                .or_insert(0);
            *entry += 1;
            self.push_diagnostic(
                "capability-pack",
                &execution.pack_id,
                ExtensionDiagnosticSeverity::Info,
                ExtensionDiagnosticKind::Migration,
                "migration_applied",
                format!(
                    "applied migration `{}` at order {}",
                    execution.migration_id, execution.order
                ),
            );
        }

        for (pack_id, total_steps) in steps_per_pack {
            if executed_per_pack.get(&pack_id).copied().unwrap_or_default() == total_steps {
                self.migrated_capability_packs.insert(pack_id);
            }
        }

        if let Some(failure) = report.failure.as_ref() {
            self.push_diagnostic(
                "capability-pack",
                &failure.pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Migration,
                "migration_failed",
                format!(
                    "migration `{}` failed at order {}: {}",
                    failure.migration_id, failure.order, failure.reason
                ),
            );
        }

        report
    }

    // --- Stage / ingester resolution & readiness gates ---

    pub fn resolve_stage_owner_for_execution(
        &self,
        stage_id: &str,
    ) -> Result<&str, CoreExtensionHostError> {
        let Some(pack_id) = self.capability_packs.resolve_stage_owner(stage_id) else {
            return Err(CoreExtensionHostError::CapabilityStageNotRegistered(
                stage_id.to_string(),
            ));
        };
        self.ensure_capability_pack_ready(pack_id)?;
        Ok(pack_id)
    }

    pub fn resolve_ingester_owner_for_ingest(
        &self,
        ingester_id: &str,
    ) -> Result<&str, CoreExtensionHostError> {
        let Some(pack_id) = self.capability_packs.resolve_ingester_owner(ingester_id) else {
            return Err(CoreExtensionHostError::CapabilityIngesterNotRegistered(
                ingester_id.to_string(),
            ));
        };
        self.ensure_capability_pack_ready(pack_id)?;
        Ok(pack_id)
    }

    pub fn ensure_capability_pack_ready(
        &self,
        capability_pack_id: &str,
    ) -> Result<(), CoreExtensionHostError> {
        let Some(report) = self.capability_readiness_report(capability_pack_id) else {
            return Err(CoreExtensionHostError::CapabilityNotReady {
                capability_pack_id: capability_pack_id.to_string(),
                reason: "capability pack is not registered".to_string(),
            });
        };

        if report.status == ExtensionReadinessStatus::Ready {
            return Ok(());
        }

        let reason = if report.failures.is_empty() {
            "readiness checks reported a failure".to_string()
        } else {
            report
                .failures
                .iter()
                .map(|failure| format!("{}: {}", failure.code, failure.message))
                .collect::<Vec<_>>()
                .join("; ")
        };

        Err(CoreExtensionHostError::CapabilityNotReady {
            capability_pack_id: report.id,
            reason,
        })
    }

    // --- Registries, diagnostics, readiness snapshot ---

    pub fn language_packs(&self) -> &LanguagePackRegistry {
        &self.language_packs
    }

    pub fn language_packs_mut(&mut self) -> &mut LanguagePackRegistry {
        &mut self.language_packs
    }

    pub fn capability_packs(&self) -> &CapabilityPackRegistry {
        &self.capability_packs
    }

    pub fn capability_packs_mut(&mut self) -> &mut CapabilityPackRegistry {
        &mut self.capability_packs
    }

    pub fn diagnostics(&self) -> &[ExtensionDiagnostic] {
        &self.diagnostics
    }

    pub fn applied_migrations(&self) -> &[CapabilityMigrationExecution] {
        &self.applied_migrations
    }

    pub fn readiness_snapshot(&self) -> CoreExtensionHostReadinessSnapshot {
        let readiness_reports = self.readiness_reports();
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.extend(self.readiness_diagnostics(&readiness_reports));

        CoreExtensionHostReadinessSnapshot {
            language_pack_ids: self
                .language_packs
                .registered_pack_ids()
                .into_iter()
                .map(str::to_string)
                .collect(),
            language_adapter_pack_ids: Vec::new(),
            capability_pack_ids: self
                .capability_packs
                .registered_pack_ids()
                .into_iter()
                .map(str::to_string)
                .collect(),
            language_observations: self.language_packs.observations().to_vec(),
            capability_observations: self.capability_packs.observations().to_vec(),
            diagnostics,
            language_adapter_readiness_reports: Vec::new(),
            readiness_reports,
        }
    }

    fn readiness_reports(&self) -> Vec<ExtensionReadinessReport> {
        let mut reports = Vec::new();

        for pack_id in self.language_packs.registered_pack_ids() {
            reports.push(ExtensionReadinessReport {
                family: "language-pack".to_string(),
                id: pack_id.to_string(),
                registered: true,
                ready: true,
                status: ExtensionReadinessStatus::Ready,
                lifecycle_state: ExtensionLifecycleState::Ready,
                failures: Vec::new(),
            });
        }

        for pack_id in self.capability_packs.registered_pack_ids() {
            let Some(descriptor) = self.capability_packs.resolve_pack(pack_id) else {
                continue;
            };
            let health_context =
                self.capability_health_context(pack_id, descriptor.migrations.len());
            let failures = self.evaluate_capability_health(&health_context);
            let status = ExtensionReadinessStatus::from_failures(!failures.is_empty());
            reports.push(ExtensionReadinessReport {
                family: "capability-pack".to_string(),
                id: pack_id.to_string(),
                registered: true,
                ready: status == ExtensionReadinessStatus::Ready,
                status,
                lifecycle_state: if status == ExtensionReadinessStatus::Ready {
                    if descriptor.migrations.is_empty() {
                        ExtensionLifecycleState::Ready
                    } else {
                        ExtensionLifecycleState::Migrated
                    }
                } else {
                    ExtensionLifecycleState::Registered
                },
                failures,
            });
        }

        reports
    }

    fn capability_readiness_report(
        &self,
        capability_pack_id: &str,
    ) -> Option<ExtensionReadinessReport> {
        let resolved_pack_id = self
            .capability_packs
            .resolve_pack(capability_pack_id)?
            .id()
            .to_ascii_lowercase();
        self.readiness_reports()
            .into_iter()
            .find(|report| report.family == "capability-pack" && report.id == resolved_pack_id)
    }

    fn capability_health_context(
        &self,
        capability_pack_id: &str,
        migration_count: usize,
    ) -> CapabilityHealthContext {
        CapabilityHealthContext::new(
            capability_pack_id,
            self.compatibility_context.runtime.as_str(),
            true,
            self.migrated_capability_packs.contains(capability_pack_id),
            migration_count,
        )
    }

    fn evaluate_capability_health(
        &self,
        context: &CapabilityHealthContext,
    ) -> Vec<ExtensionReadinessFailure> {
        let mut failures = Vec::new();
        if context.has_pending_migrations() {
            failures.push(ExtensionReadinessFailure {
                code: "migrations_pending".to_string(),
                message: format!(
                    "capability pack has unapplied migrations in runtime `{}`",
                    context.runtime
                ),
            });
        }
        failures
    }

    fn push_diagnostic(
        &mut self,
        family: &str,
        extension_id: &str,
        severity: ExtensionDiagnosticSeverity,
        kind: ExtensionDiagnosticKind,
        code: &str,
        message: String,
    ) {
        self.diagnostics.push(ExtensionDiagnostic {
            family: family.to_string(),
            extension_id: extension_id.to_string(),
            severity,
            kind,
            code: code.to_string(),
            message,
        });
    }

    fn readiness_diagnostics(
        &self,
        readiness_reports: &[ExtensionReadinessReport],
    ) -> Vec<ExtensionDiagnostic> {
        let mut diagnostics = Vec::new();
        for report in readiness_reports {
            for failure in &report.failures {
                diagnostics.push(ExtensionDiagnostic {
                    family: report.family.clone(),
                    extension_id: report.id.clone(),
                    severity: ExtensionDiagnosticSeverity::Error,
                    kind: ExtensionDiagnosticKind::Readiness,
                    code: failure.code.clone(),
                    message: failure.message.clone(),
                });
            }
        }
        diagnostics
    }

    /// Serializable registry snapshot (language packs, extension capability descriptors, migration plan, readiness, diagnostics).
    pub fn registry_report(&self) -> registry_report::CoreExtensionHostRegistryReport {
        registry_report::build(self)
    }

    pub fn registry_report_with_snapshot(
        &self,
        snapshot: CoreExtensionHostReadinessSnapshot,
    ) -> registry_report::CoreExtensionHostRegistryReport {
        registry_report::build_with_snapshot(self, snapshot)
    }

    /// Capability pack ids that have completed migrations in this host instance (empty until `run_capability_migrations` runs).
    pub fn migrated_capability_pack_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.migrated_capability_packs.iter().cloned().collect();
        ids.sort_unstable();
        ids
    }
}
