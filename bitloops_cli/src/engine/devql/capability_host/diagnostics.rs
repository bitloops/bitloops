//! Serializable registry / lifecycle snapshots for `DevqlCapabilityHost` (CLI and tooling).

use serde::Serialize;

use super::health::CapabilityHealthResult;
use super::host::DevqlCapabilityHost;
use super::policy::{CrossPackGrant, HostInvocationPolicy, PackTrustTier};

/// Full host registry snapshot (optionally extended with [`HealthOutcome`]).
#[derive(Debug, Clone, Serialize)]
pub struct HostRegistryReport {
    pub repo_id: String,
    pub repo_identity: String,
    pub repo_root: String,
    pub migrations_applied_this_session: bool,
    pub invocation: InvocationSummary,
    pub cross_pack_grants: Vec<CrossPackGrantSummary>,
    /// All registered migrations in host execution order (may include multiple packs).
    pub migration_plan: Vec<MigrationStepSummary>,
    pub packs: Vec<PackRegistryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health: Vec<HealthOutcome>,
}

/// DevQL capability host plus optional [`CoreExtensionHostRegistryReport`](crate::engine::extensions::CoreExtensionHostRegistryReport) (same CLI).
#[derive(Debug, Clone, Serialize)]
pub struct PackLifecycleReport {
    pub devql_capability_host: HostRegistryReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_extension_host: Option<crate::engine::extensions::CoreExtensionHostRegistryReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_extension_host_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvocationSummary {
    pub stage_timeout_secs: u64,
    pub ingester_timeout_secs: u64,
    pub subquery_timeout_secs: u64,
    pub trust_tier: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossPackGrantSummary {
    pub from_capability: String,
    pub to_capability: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationStepSummary {
    pub capability_id: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaModuleSummary {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackRegistryEntry {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub api_version: u32,
    pub default_enabled: bool,
    pub experimental: bool,
    pub dependencies: Vec<String>,
    pub stages: Vec<String>,
    pub ingesters: Vec<String>,
    pub migrations: Vec<MigrationStepSummary>,
    pub schema_modules: Vec<SchemaModuleSummary>,
    pub health_check_names: Vec<String>,
    pub query_example_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthOutcome {
    pub check_id: String,
    pub healthy: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl From<&HostInvocationPolicy> for InvocationSummary {
    fn from(policy: &HostInvocationPolicy) -> Self {
        Self {
            stage_timeout_secs: policy.stage_timeout.as_secs(),
            ingester_timeout_secs: policy.ingester_timeout.as_secs(),
            subquery_timeout_secs: policy.subquery_timeout.as_secs(),
            trust_tier: match policy.trust_tier {
                PackTrustTier::FirstParty => "first_party".to_string(),
                PackTrustTier::ThirdParty => "third_party".to_string(),
            },
        }
    }
}

impl From<&CrossPackGrant> for CrossPackGrantSummary {
    fn from(g: &CrossPackGrant) -> Self {
        Self {
            from_capability: g.from_capability.clone(),
            to_capability: g.to_capability.clone(),
            resource: g.resource.clone(),
        }
    }
}

/// Run all registered pack health checks (uses public `DevqlCapabilityHost::run_health_checks`).
pub fn collect_health_outcomes(host: &DevqlCapabilityHost) -> Vec<HealthOutcome> {
    let mut ids: Vec<String> = host.descriptors().map(|d| d.id.to_string()).collect();
    ids.sort();
    let mut out = Vec::new();
    for id in ids {
        for (check_id, result) in host.run_health_checks(&id) {
            out.push(HealthOutcome::from_result(&check_id, &result));
        }
    }
    out
}

impl HealthOutcome {
    fn from_result(check_id: &str, result: &CapabilityHealthResult) -> Self {
        Self {
            check_id: check_id.to_string(),
            healthy: result.is_healthy(),
            message: result.message.clone(),
            details: result.details.clone(),
        }
    }
}

pub fn format_registry_report_human(report: &HostRegistryReport) -> String {
    use std::fmt::Write;

    let mut s = String::new();

    writeln!(s, "DevQL capability host — registry & lifecycle").ok();
    writeln!(s, "  repo: {}", report.repo_identity).ok();
    writeln!(s, "  repo_id: {}", report.repo_id).ok();
    writeln!(s, "  repo_root: {}", report.repo_root).ok();
    writeln!(
        s,
        "  migrations_applied (this process): {}",
        if report.migrations_applied_this_session {
            "yes"
        } else {
            "no (run ingest/init or pass --apply-migrations)"
        }
    )
    .ok();
    writeln!(
        s,
        "  invocation: stage_timeout={}s ingester_timeout={}s subquery_timeout={}s trust={}",
        report.invocation.stage_timeout_secs,
        report.invocation.ingester_timeout_secs,
        report.invocation.subquery_timeout_secs,
        report.invocation.trust_tier
    )
    .ok();

    if report.cross_pack_grants.is_empty() {
        writeln!(s, "  cross_pack_access grants: (none)").ok();
    } else {
        writeln!(
            s,
            "  cross_pack_access grants: {}",
            report.cross_pack_grants.len()
        )
        .ok();
        for g in &report.cross_pack_grants {
            writeln!(
                s,
                "    - {} -> {} ({})",
                g.from_capability, g.to_capability, g.resource
            )
            .ok();
        }
    }

    writeln!(
        s,
        "  migration_plan ({} steps):",
        report.migration_plan.len()
    )
    .ok();
    for step in &report.migration_plan {
        writeln!(
            s,
            "    - [{}] v{} — {}",
            step.capability_id, step.version, step.description
        )
        .ok();
    }

    writeln!(s, "  packs ({}):", report.packs.len()).ok();
    for pack in &report.packs {
        writeln!(s).ok();
        writeln!(
            s,
            "  [{}] {} — v{} (api {}){}",
            pack.id,
            pack.display_name,
            pack.version,
            pack.api_version,
            if pack.experimental {
                " [experimental]"
            } else {
                ""
            }
        )
        .ok();
        if !pack.dependencies.is_empty() {
            writeln!(s, "      dependencies: {}", pack.dependencies.join(", ")).ok();
        }
        writeln!(
            s,
            "      stages ({}): {}",
            pack.stages.len(),
            if pack.stages.is_empty() {
                "(none)".to_string()
            } else {
                pack.stages.join(", ")
            }
        )
        .ok();
        writeln!(
            s,
            "      ingesters ({}): {}",
            pack.ingesters.len(),
            if pack.ingesters.is_empty() {
                "(none)".to_string()
            } else {
                pack.ingesters.join(", ")
            }
        )
        .ok();
        if pack.migrations.is_empty() {
            writeln!(s, "      migrations: (none registered)").ok();
        } else {
            writeln!(s, "      migrations:").ok();
            for m in &pack.migrations {
                writeln!(s, "        - v{} — {}", m.version, m.description).ok();
            }
        }
        if pack.schema_modules.is_empty() {
            writeln!(s, "      schema_modules: (none)").ok();
        } else {
            writeln!(s, "      schema_modules:").ok();
            for sm in &pack.schema_modules {
                writeln!(s, "        - {} — {}", sm.name, sm.description).ok();
            }
        }
        writeln!(
            s,
            "      health_checks ({}): {}",
            pack.health_check_names.len(),
            if pack.health_check_names.is_empty() {
                "(none)".to_string()
            } else {
                pack.health_check_names.join(", ")
            }
        )
        .ok();
        writeln!(s, "      query_examples: {}", pack.query_example_count).ok();
    }

    if !report.health.is_empty() {
        writeln!(s).ok();
        writeln!(s, "  health ({} checks):", report.health.len()).ok();
        for h in &report.health {
            let status = if h.healthy { "ok" } else { "fail" };
            writeln!(s, "    [{status}] {} — {}", h.check_id, h.message).ok();
            if let Some(d) = &h.details {
                for line in d.lines() {
                    writeln!(s, "         {line}").ok();
                }
            }
        }
    }

    s
}

pub fn format_pack_lifecycle_report_human(report: &PackLifecycleReport) -> String {
    let mut s = format_registry_report_human(&report.devql_capability_host);
    if let Some(ext) = report.core_extension_host.as_ref() {
        s.push_str("\n---\n");
        s.push_str(&crate::engine::extensions::format_core_extension_host_registry_human(ext));
    } else if let Some(err) = report.core_extension_host_error.as_ref() {
        s.push_str("\n---\nCore extension host: snapshot unavailable: ");
        s.push_str(err);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::devql::RepoIdentity;
    use crate::engine::devql::capability_host::DevqlCapabilityHost;
    use tempfile::tempdir;

    fn sample_repo() -> RepoIdentity {
        RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "diag-test".to_string(),
            identity: "github/bitloops/diag-test".to_string(),
            repo_id: "repo-id-diag-test".to_string(),
        }
    }

    #[test]
    fn builtin_registry_report_lists_three_first_party_packs() {
        let tmp = tempdir().expect("tempdir");
        let host = DevqlCapabilityHost::builtin(tmp.path(), sample_repo()).expect("builtin host");
        let report = host.registry_report();
        let mut ids: Vec<_> = report.packs.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["knowledge", "semantic_clones", "test_harness"]);
        assert!(
            !report.migration_plan.is_empty(),
            "expected at least one pack migration registered"
        );
    }

    #[test]
    fn human_report_is_non_empty() {
        let tmp = tempdir().expect("tempdir");
        let host = DevqlCapabilityHost::builtin(tmp.path(), sample_repo()).expect("builtin host");
        let report = host.registry_report();
        let text = format_registry_report_human(&report);
        assert!(text.contains("knowledge"));
        assert!(text.contains("semantic_clones"));
        assert!(text.contains("test_harness"));
        assert!(text.contains("migration_plan"));
    }
}
