//! Serializable registry snapshot for [`super::CoreExtensionHost`] (CLI / diagnostics).

use serde::Serialize;

use crate::host::extension_host::capability::CapabilityPackRegistrationStatus;
use crate::host::extension_host::language::LanguagePackRegistrationStatus;
use crate::host::extension_host::lifecycle::{
    ExtensionDiagnosticKind, ExtensionDiagnosticSeverity, ExtensionLifecycleState,
    ExtensionReadinessStatus,
};

use super::CoreExtensionHost;

/// `CoreExtensionHost` registry: language + extension capability descriptors, migration plan, readiness, diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct CoreExtensionHostRegistryReport {
    pub subsystem: &'static str,
    pub compatibility: ExtensionHostCompatibilityJson,
    pub language_packs: Vec<ExtensionLanguagePackJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub language_adapter_pack_ids: Vec<String>,
    pub capability_packs: Vec<ExtensionCapabilityPackJson>,
    pub capability_migration_plan: Vec<ExtensionCapabilityMigrationStepJson>,
    pub migrated_capability_pack_ids: Vec<String>,
    pub applied_capability_migrations: Vec<ExtensionAppliedCapabilityMigrationJson>,
    pub readiness: Vec<ExtensionReadinessJson>,
    pub registration_observations: ExtensionRegistrationObservationsJson,
    pub diagnostics: Vec<ExtensionDiagnosticJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionHostCompatibilityJson {
    pub contract_version: u16,
    pub runtime_version: u16,
    pub runtime: String,
    pub supported_features: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionLanguagePackJson {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub api_version: u32,
    pub aliases: Vec<String>,
    pub supported_languages: Vec<String>,
    pub profile_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionCapabilityPackJson {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub api_version: u32,
    pub default_enabled: bool,
    pub experimental: bool,
    pub aliases: Vec<String>,
    pub dependencies: Vec<String>,
    pub stages: Vec<String>,
    pub ingesters: Vec<String>,
    pub schema_modules: Vec<String>,
    pub query_examples: Vec<ExtensionQueryExampleJson>,
    pub migration_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionQueryExampleJson {
    pub id: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionCapabilityMigrationStepJson {
    pub pack_id: String,
    pub migration_id: String,
    pub order: u32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionAppliedCapabilityMigrationJson {
    pub pack_id: String,
    pub migration_id: String,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionReadinessJson {
    pub family: String,
    pub id: String,
    pub registered: bool,
    pub ready: bool,
    pub status: String,
    pub lifecycle_state: String,
    pub failures: Vec<ExtensionReadinessFailureJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionReadinessFailureJson {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionRegistrationObservationsJson {
    pub language: Vec<ExtensionObservationJson>,
    pub capability: Vec<ExtensionObservationJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionObservationJson {
    pub pack_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionDiagnosticJson {
    pub family: String,
    pub extension_id: String,
    pub severity: String,
    pub kind: String,
    pub code: String,
    pub message: String,
}

pub fn build_with_snapshot(
    host: &CoreExtensionHost,
    snapshot: super::CoreExtensionHostReadinessSnapshot,
) -> CoreExtensionHostRegistryReport {
    let ctx = host.compatibility_context();
    let compatibility = ExtensionHostCompatibilityJson {
        contract_version: ctx.contract_version,
        runtime_version: ctx.runtime_version,
        runtime: ctx.runtime.as_str().to_string(),
        supported_features: ctx
            .supported_features
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    };

    let mut language_packs = Vec::new();
    for pack_id in host.language_packs().registered_pack_ids() {
        let Some(d) = host.language_packs().resolve_pack(pack_id) else {
            continue;
        };
        let profile_ids = d
            .language_profiles
            .iter()
            .map(|p| p.id.to_string())
            .collect();
        language_packs.push(ExtensionLanguagePackJson {
            id: d.id.to_string(),
            display_name: d.display_name.to_string(),
            version: d.version.to_string(),
            api_version: d.api_version,
            aliases: d.aliases.iter().map(|s| (*s).to_string()).collect(),
            supported_languages: d
                .supported_languages
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            profile_ids,
        });
    }
    language_packs.sort_by(|a, b| a.id.cmp(&b.id));

    let mut capability_packs = Vec::new();
    for pack_id in host.capability_packs().registered_pack_ids() {
        let Some(d) = host.capability_packs().resolve_pack(pack_id) else {
            continue;
        };
        let c = &d.capability;
        let query_examples = d
            .query_example_contributions
            .iter()
            .map(|q| ExtensionQueryExampleJson {
                id: q.id.to_string(),
                query: q.query.to_string(),
            })
            .collect();
        capability_packs.push(ExtensionCapabilityPackJson {
            id: c.id.to_string(),
            display_name: c.display_name.to_string(),
            version: c.version.to_string(),
            api_version: c.api_version,
            default_enabled: c.default_enabled,
            experimental: c.experimental,
            aliases: d.aliases.iter().map(|s| (*s).to_string()).collect(),
            dependencies: c
                .dependencies
                .iter()
                .map(|dep| format!("{} (>={})", dep.capability_id, dep.min_version))
                .collect(),
            stages: d
                .stage_contributions
                .iter()
                .map(|s| s.id.to_string())
                .collect(),
            ingesters: d
                .ingester_contributions
                .iter()
                .map(|s| s.id.to_string())
                .collect(),
            schema_modules: d
                .schema_module_contributions
                .iter()
                .map(|s| s.id.to_string())
                .collect(),
            query_examples,
            migration_count: d.migrations.len(),
        });
    }
    capability_packs.sort_by(|a, b| a.id.cmp(&b.id));

    let capability_migration_plan: Vec<ExtensionCapabilityMigrationStepJson> = host
        .capability_migration_plan()
        .into_iter()
        .map(|step| ExtensionCapabilityMigrationStepJson {
            pack_id: step.pack_id,
            migration_id: step.migration_id,
            order: step.order,
            description: step.description,
        })
        .collect();

    let mut migrated_capability_pack_ids = host.migrated_capability_pack_ids();
    migrated_capability_pack_ids.sort();

    let mut applied_capability_migrations: Vec<ExtensionAppliedCapabilityMigrationJson> = host
        .applied_migrations()
        .iter()
        .map(|e| ExtensionAppliedCapabilityMigrationJson {
            pack_id: e.pack_id.clone(),
            migration_id: e.migration_id.clone(),
            order: e.order,
        })
        .collect();
    applied_capability_migrations.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.pack_id.cmp(&b.pack_id))
            .then_with(|| a.migration_id.cmp(&b.migration_id))
    });

    let readiness: Vec<ExtensionReadinessJson> = snapshot
        .readiness_reports
        .iter()
        .map(|r| ExtensionReadinessJson {
            family: r.family.clone(),
            id: r.id.clone(),
            registered: r.registered,
            ready: r.ready,
            status: readiness_status_str(r.status),
            lifecycle_state: lifecycle_state_str(r.lifecycle_state),
            failures: r
                .failures
                .iter()
                .map(|f| ExtensionReadinessFailureJson {
                    code: f.code.clone(),
                    message: f.message.clone(),
                })
                .collect(),
        })
        .collect();

    let registration_observations = ExtensionRegistrationObservationsJson {
        language: snapshot
            .language_observations
            .iter()
            .map(|o| ExtensionObservationJson {
                pack_id: o.pack_id.clone(),
                status: language_obs_status(&o.status),
                reason: o.reason.clone(),
            })
            .collect(),
        capability: snapshot
            .capability_observations
            .iter()
            .map(|o| ExtensionObservationJson {
                pack_id: o.pack_id.clone(),
                status: capability_obs_status(&o.status),
                reason: o.reason.clone(),
            })
            .collect(),
    };

    let diagnostics: Vec<ExtensionDiagnosticJson> = snapshot
        .diagnostics
        .iter()
        .map(|d| ExtensionDiagnosticJson {
            family: d.family.clone(),
            extension_id: d.extension_id.clone(),
            severity: diagnostic_severity_str(d.severity),
            kind: diagnostic_kind_str(d.kind),
            code: d.code.clone(),
            message: d.message.clone(),
        })
        .collect();

    CoreExtensionHostRegistryReport {
        subsystem: "core_extension_host",
        compatibility,
        language_packs,
        language_adapter_pack_ids: snapshot.language_adapter_pack_ids,
        capability_packs,
        capability_migration_plan,
        migrated_capability_pack_ids,
        applied_capability_migrations,
        readiness,
        registration_observations,
        diagnostics,
    }
}

pub fn build(host: &CoreExtensionHost) -> CoreExtensionHostRegistryReport {
    build_with_snapshot(host, host.readiness_snapshot())
}

pub fn format_core_extension_host_registry_human(
    report: &CoreExtensionHostRegistryReport,
) -> String {
    use std::fmt::Write;

    let mut s = String::new();
    writeln!(s, "Core extension host — registry & readiness").ok();
    writeln!(
        s,
        "  compatibility: contract={} runtime={} {} (features: {})",
        report.compatibility.contract_version,
        report.compatibility.runtime_version,
        report.compatibility.runtime,
        report.compatibility.supported_features.join(", ")
    )
    .ok();

    writeln!(
        s,
        "  language_packs ({}): {}",
        report.language_packs.len(),
        report
            .language_packs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .ok();
    for lp in &report.language_packs {
        writeln!(s, "    [{}] {} v{}", lp.id, lp.display_name, lp.version).ok();
        writeln!(s, "        profiles: {}", lp.profile_ids.join(", ")).ok();
    }

    if !report.language_adapter_pack_ids.is_empty() {
        writeln!(
            s,
            "  language_adapter_packs ({}): {}",
            report.language_adapter_pack_ids.len(),
            report.language_adapter_pack_ids.join(", ")
        )
        .ok();
    }

    writeln!(
        s,
        "  capability_packs ({}): {}",
        report.capability_packs.len(),
        report
            .capability_packs
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .ok();
    for cp in &report.capability_packs {
        writeln!(
            s,
            "    [{}] {} v{} (migrations declared: {})",
            cp.id, cp.display_name, cp.version, cp.migration_count
        )
        .ok();
        writeln!(s, "        stages: {}", cp.stages.join(", ")).ok();
        writeln!(s, "        ingesters: {}", cp.ingesters.join(", ")).ok();
        writeln!(
            s,
            "        schema_modules: {}",
            cp.schema_modules.join(", ")
        )
        .ok();
    }

    writeln!(
        s,
        "  capability_migration_plan ({} steps)",
        report.capability_migration_plan.len()
    )
    .ok();
    for step in &report.capability_migration_plan {
        writeln!(
            s,
            "    - [{}] order={} id={} — {}",
            step.pack_id, step.order, step.migration_id, step.description
        )
        .ok();
    }

    writeln!(
        s,
        "  migrated_capability_pack_ids (this process): [{}]",
        report.migrated_capability_pack_ids.join(", ")
    )
    .ok();
    writeln!(
        s,
        "  applied_capability_migrations: {}",
        report.applied_capability_migrations.len()
    )
    .ok();

    writeln!(s, "  readiness:").ok();
    for r in &report.readiness {
        writeln!(
            s,
            "    [{}] {} — status={} lifecycle={} ready={}",
            r.family, r.id, r.status, r.lifecycle_state, r.ready
        )
        .ok();
        for f in &r.failures {
            writeln!(s, "      ! {}: {}", f.code, f.message).ok();
        }
    }

    if !report.diagnostics.is_empty() {
        writeln!(s, "  diagnostics ({}):", report.diagnostics.len()).ok();
        for d in &report.diagnostics {
            writeln!(
                s,
                "    [{}] [{}] {} {} — {}",
                d.severity, d.family, d.extension_id, d.code, d.message
            )
            .ok();
        }
    }

    s
}

fn readiness_status_str(status: ExtensionReadinessStatus) -> String {
    match status {
        ExtensionReadinessStatus::Ready => "ready".to_string(),
        ExtensionReadinessStatus::NotReady => "not_ready".to_string(),
    }
}

fn lifecycle_state_str(state: ExtensionLifecycleState) -> String {
    match state {
        ExtensionLifecycleState::Discovered => "discovered",
        ExtensionLifecycleState::Validated => "validated",
        ExtensionLifecycleState::Registered => "registered",
        ExtensionLifecycleState::Migrated => "migrated",
        ExtensionLifecycleState::Ready => "ready",
        ExtensionLifecycleState::Failed => "failed",
    }
    .to_string()
}

fn diagnostic_severity_str(sev: ExtensionDiagnosticSeverity) -> String {
    match sev {
        ExtensionDiagnosticSeverity::Info => "info",
        ExtensionDiagnosticSeverity::Warning => "warning",
        ExtensionDiagnosticSeverity::Error => "error",
    }
    .to_string()
}

fn diagnostic_kind_str(kind: ExtensionDiagnosticKind) -> String {
    match kind {
        ExtensionDiagnosticKind::Registration => "registration",
        ExtensionDiagnosticKind::Compatibility => "compatibility",
        ExtensionDiagnosticKind::Readiness => "readiness",
        ExtensionDiagnosticKind::Migration => "migration",
    }
    .to_string()
}

fn language_obs_status(status: &LanguagePackRegistrationStatus) -> String {
    match status {
        LanguagePackRegistrationStatus::Registered => "registered".to_string(),
        LanguagePackRegistrationStatus::Rejected => "rejected".to_string(),
    }
}

fn capability_obs_status(status: &CapabilityPackRegistrationStatus) -> String {
    match status {
        CapabilityPackRegistrationStatus::Registered => "registered".to_string(),
        CapabilityPackRegistrationStatus::Rejected => "rejected".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::extension_host::CoreExtensionHost;

    #[test]
    fn builtin_extension_registry_lists_language_and_capability_packs() {
        let host = CoreExtensionHost::with_builtins().expect("builtins");
        let r = build(&host);
        assert_eq!(r.subsystem, "core_extension_host");
        assert_eq!(r.language_packs.len(), 5);
        let lang_ids: Vec<_> = r.language_packs.iter().map(|p| p.id.as_str()).collect();
        assert!(lang_ids.contains(&"go-language-pack"));
        assert!(lang_ids.contains(&"java-language-pack"));
        assert!(lang_ids.contains(&"rust-language-pack"));
        assert!(lang_ids.contains(&"ts-js-language-pack"));
        assert!(lang_ids.contains(&"python-language-pack"));
        assert_eq!(r.capability_packs.len(), 2);
        let cap_ids: Vec<_> = r.capability_packs.iter().map(|p| p.id.as_str()).collect();
        assert!(cap_ids.contains(&"knowledge-capability-pack"));
        assert!(cap_ids.contains(&"test-harness-capability-pack"));
        let human = format_core_extension_host_registry_human(&r);
        assert!(human.contains("knowledge-capability-pack"));
    }
}
