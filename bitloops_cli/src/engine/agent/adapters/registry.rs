use anyhow::{Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;
use uuid::Uuid;

use super::builtin::builtin_registrations;
use super::registration::AgentAdapterRegistration;
use super::types::{
    AgentAdapterConfiguration, AgentAdapterPackageDiscovery, AgentAdapterReadiness,
    AgentAdapterRuntime, AgentProtocolFamilyDescriptor, AgentReadinessFailure,
    AgentReadinessStatus, AgentRegistrationObservation, AgentResolutionTrace,
    AgentResolvedRegistration, AgentTargetProfileDescriptor, AliasResolutionSource, normalise_key,
};

pub struct AgentAdapterRegistry {
    registrations: HashMap<String, AgentAdapterRegistration>,
    aliases: HashMap<String, String>,
    alias_sources: HashMap<String, AliasResolutionSource>,
    profile_aliases: HashMap<String, String>,
    ordered_ids: Vec<String>,
    default_id: String,
    families: HashMap<String, AgentProtocolFamilyDescriptor>,
    profiles: HashMap<String, AgentTargetProfileDescriptor>,
    profile_to_adapter: HashMap<String, String>,
}

impl AgentAdapterRegistry {
    pub fn new(registrations: Vec<AgentAdapterRegistration>) -> Result<Self> {
        if registrations.is_empty() {
            bail!("at least one adapter registration is required");
        }

        let mut map: HashMap<String, AgentAdapterRegistration> = HashMap::new();
        let mut aliases: HashMap<String, String> = HashMap::new();
        let mut alias_sources: HashMap<String, AliasResolutionSource> = HashMap::new();
        let mut profile_aliases: HashMap<String, String> = HashMap::new();
        let mut ordered_ids = Vec::new();
        let mut default_id: Option<String> = None;
        let mut used_agent_types: HashSet<String> = HashSet::new();
        let mut families: HashMap<String, AgentProtocolFamilyDescriptor> = HashMap::new();
        let mut profiles: HashMap<String, AgentTargetProfileDescriptor> = HashMap::new();
        let mut profile_to_adapter: HashMap<String, String> = HashMap::new();

        for registration in registrations {
            let id = normalise_key(registration.descriptor().id)?;
            let descriptor = registration.descriptor();

            descriptor.compatibility.validate(&id, "adapter")?;
            descriptor.runtime.validate(&id, "adapter")?;
            descriptor.config_schema.validate_shape("adapter", &id)?;
            descriptor.package.validate("package", &id)?;

            if descriptor.package.display_name.trim() != descriptor.display_name.trim() {
                bail!(
                    "package {} must share the adapter display name {}",
                    descriptor.package.id,
                    descriptor.display_name,
                );
            }

            let agent_type = normalise_key(descriptor.agent_type)?;
            if !used_agent_types.insert(agent_type.clone()) {
                bail!("duplicate adapter agent type: {agent_type}");
            }

            if map.contains_key(&id) {
                bail!("duplicate adapter id: {id}");
            }

            if descriptor.is_default {
                if default_id.is_some() {
                    bail!("multiple default adapters configured");
                }
                default_id = Some(id.clone());
            }

            let family_id = normalise_key(descriptor.protocol_family.id)?;
            descriptor
                .protocol_family
                .compatibility
                .validate(&family_id, "protocol family")?;
            descriptor
                .protocol_family
                .runtime
                .validate(&family_id, "protocol family")?;
            descriptor
                .protocol_family
                .config_schema
                .validate_shape("protocol family", &family_id)?;

            if let Some(existing) = families.get(&family_id) {
                validate_family_descriptor(existing, &descriptor.protocol_family, &family_id)?;
            } else {
                families.insert(family_id.clone(), descriptor.protocol_family.clone());
            }

            let profile_id = normalise_key(descriptor.target_profile.id)?;
            let profile_family_id = normalise_key(descriptor.target_profile.family_id)?;
            if profile_family_id != family_id {
                bail!(
                    "target profile {} refers to family {} but adapter {} belongs to family {}",
                    descriptor.target_profile.id,
                    descriptor.target_profile.family_id,
                    descriptor.id,
                    descriptor.protocol_family.id,
                );
            }

            descriptor
                .target_profile
                .compatibility
                .validate(&profile_id, "target profile")?;
            descriptor
                .target_profile
                .runtime
                .validate(&profile_id, "target profile")?;
            descriptor
                .target_profile
                .config_schema
                .validate_shape("target profile", &profile_id)?;

            if let Some(existing) = profiles.get(&profile_id) {
                validate_profile_descriptor(existing, &descriptor.target_profile, &profile_id)?;
            } else {
                profiles.insert(profile_id.clone(), descriptor.target_profile.clone());
            }

            if let Some(existing) = profile_to_adapter.get(&profile_id)
                && existing != &id
            {
                bail!(
                    "target profile {} is already mapped to adapter {}",
                    descriptor.target_profile.id,
                    existing
                );
            }
            profile_to_adapter.insert(profile_id.clone(), id.clone());

            register_alias(
                &mut aliases,
                &mut alias_sources,
                &id,
                descriptor.id,
                AliasResolutionSource::LegacyTarget,
            )?;
            for alias in descriptor.aliases {
                register_alias(
                    &mut aliases,
                    &mut alias_sources,
                    &id,
                    alias,
                    AliasResolutionSource::LegacyTarget,
                )?;
            }

            register_profile_alias(
                &mut profile_aliases,
                &profile_id,
                descriptor.target_profile.id,
            )?;
            for alias in descriptor.target_profile.aliases {
                register_profile_alias(&mut profile_aliases, &profile_id, alias)?;
                register_alias(
                    &mut aliases,
                    &mut alias_sources,
                    &id,
                    alias,
                    AliasResolutionSource::TargetProfile,
                )?;
            }

            register_alias(
                &mut aliases,
                &mut alias_sources,
                &id,
                descriptor.target_profile.id,
                AliasResolutionSource::TargetProfile,
            )?;

            ordered_ids.push(id.clone());
            map.insert(id, registration);
        }

        let Some(default_id) = default_id else {
            bail!("no default adapter configured");
        };

        Ok(Self {
            registrations: map,
            aliases,
            alias_sources,
            profile_aliases,
            ordered_ids,
            default_id,
            families,
            profiles,
            profile_to_adapter,
        })
    }

    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<AgentAdapterRegistry> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            AgentAdapterRegistry::new(builtin_registrations())
                .expect("builtin adapter registrations must be valid")
        })
    }

    pub fn available_agents(&self) -> Vec<String> {
        self.ordered_ids
            .iter()
            .map(|id| {
                self.registrations
                    .get(id)
                    .expect("adapter id missing from registry")
                    .descriptor()
                    .id
                    .to_string()
            })
            .collect()
    }

    pub fn available_protocol_families(&self) -> Vec<String> {
        let mut ids = self.families.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn available_target_profiles(&self) -> Vec<String> {
        let mut ids = self.profiles.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub fn discover_packages(&self) -> Vec<AgentAdapterPackageDiscovery> {
        self.ordered_ids
            .iter()
            .filter_map(|id| {
                self.registrations.get(id).map(|registration| {
                    registration
                        .descriptor()
                        .package
                        .discovery_report("package", registration.descriptor().id)
                })
            })
            .collect()
    }

    pub fn validate_package_metadata(&self) -> Vec<AgentAdapterPackageDiscovery> {
        self.discover_packages()
    }

    pub fn package_discovery_reports(&self) -> Vec<AgentAdapterPackageDiscovery> {
        self.discover_packages()
    }

    pub fn package_validation_reports(&self) -> Vec<AgentAdapterPackageDiscovery> {
        self.discover_packages()
    }

    pub fn protocol_families(&self) -> Vec<AgentProtocolFamilyDescriptor> {
        let mut families = self.families.values().cloned().collect::<Vec<_>>();
        families.sort_by(|left, right| left.id.cmp(right.id));
        families
    }

    pub fn target_profiles(&self) -> Vec<AgentTargetProfileDescriptor> {
        let mut profiles = self.profiles.values().cloned().collect::<Vec<_>>();
        profiles.sort_by(|left, right| left.id.cmp(right.id));
        profiles
    }

    pub fn default_agent_name(&self) -> &str {
        self.registrations
            .get(&self.default_id)
            .expect("default adapter id missing")
            .descriptor()
            .id
    }

    pub fn normalise_agent_name(&self, value: &str) -> Result<String> {
        let key = normalise_key(value)?;
        let id = self
            .aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown agent name: {}", value.trim()))?;
        Ok(self
            .registrations
            .get(id)
            .expect("resolved adapter id missing")
            .descriptor()
            .id
            .to_string())
    }

    pub fn normalise_profile_name(&self, value: &str) -> Result<String> {
        let key = normalise_key(value)?;
        let profile_id = self
            .profile_aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown target profile: {}", value.trim()))?;
        Ok(profile_id.to_string())
    }

    pub fn resolve_profile(&self, value: &str) -> Result<&AgentTargetProfileDescriptor> {
        let key = normalise_key(value)?;
        let profile_id = self
            .profile_aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown target profile: {}", value.trim()))?;
        let profile = self
            .profiles
            .get(profile_id)
            .ok_or_else(|| anyhow!("target profile metadata missing: {profile_id}"))?;

        profile
            .compatibility
            .validate(profile_id, "target profile")?;
        profile.runtime.validate(profile_id, "target profile")?;

        Ok(profile)
    }

    pub fn resolve_composed(
        &self,
        family: &str,
        profile: &str,
    ) -> Result<&AgentAdapterRegistration> {
        let family_id = normalise_key(family)?;
        let profile_descriptor = self.resolve_profile(profile)?;
        let profile_id = normalise_key(profile_descriptor.id)?;
        let profile_family_id = normalise_key(profile_descriptor.family_id)?;

        if profile_family_id != family_id {
            bail!(
                "target profile {} does not belong to protocol family {}",
                profile_descriptor.id,
                family.trim()
            );
        }

        let adapter_id = self
            .profile_to_adapter
            .get(&profile_id)
            .ok_or_else(|| anyhow!("adapter mapping missing for target profile: {profile_id}"))?;
        let registration = self
            .registrations
            .get(adapter_id)
            .ok_or_else(|| anyhow!("adapter registration missing: {adapter_id}"))?;

        self.validate_resolution_compatibility(registration)?;
        Ok(registration)
    }

    pub fn resolve(&self, value: &str) -> Result<&AgentAdapterRegistration> {
        let key = normalise_key(value)?;
        let id = self
            .aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown agent name: {}", value.trim()))?;
        let registration = self
            .registrations
            .get(id)
            .ok_or_else(|| anyhow!("adapter registration missing: {id}"))?;

        self.validate_resolution_compatibility(registration)?;
        Ok(registration)
    }

    pub fn resolve_with_trace(
        &self,
        value: &str,
        correlation_id: Option<&str>,
    ) -> Result<AgentResolvedRegistration<'_>> {
        let key = normalise_key(value)?;
        let source = self
            .alias_sources
            .get(&key)
            .copied()
            .unwrap_or(AliasResolutionSource::LegacyTarget);

        let registration = self.resolve(value)?;
        let descriptor = registration.descriptor();

        let correlation_id = correlation_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let trace = AgentResolutionTrace {
            correlation_id,
            requested: value.trim().to_string(),
            resolved_adapter_id: descriptor.id.to_string(),
            package_id: descriptor.package.id.to_string(),
            package_metadata_version: descriptor.package.metadata_version.value(),
            package_version: descriptor.package.version.to_string(),
            package_source: descriptor.package.source.as_str().to_string(),
            package_trust_model: descriptor.package.trust_model.as_str().to_string(),
            protocol_family: descriptor.protocol_family.id.to_string(),
            target_profile: descriptor.target_profile.id.to_string(),
            runtime: AgentAdapterRuntime::LocalCli.as_str().to_string(),
            used_alias: key != descriptor.id,
            resolution_path: source.as_str().to_string(),
            diagnostics: vec![
                format!("normalized_input={key}"),
                format!("resolution_source={}", source.as_str()),
                format!("package_id={}", descriptor.package.id),
                format!("package_version={}", descriptor.package.version),
                format!(
                    "package_trust_model={}",
                    descriptor.package.trust_model.as_str()
                ),
                format!("protocol_family={}", descriptor.protocol_family.id),
                format!("target_profile={}", descriptor.target_profile.id),
            ],
        };

        Ok(AgentResolvedRegistration {
            registration,
            trace,
        })
    }

    pub fn agent_display(&self, value: &str) -> Option<&'static str> {
        self.resolve(value)
            .ok()
            .map(|registration| registration.descriptor().display_name)
    }

    pub fn detect_project_agents(&self, repo_root: &Path) -> Vec<String> {
        self.ordered_ids
            .iter()
            .filter_map(|id| {
                let registration = self.registrations.get(id)?;
                if registration.is_project_detected(repo_root) {
                    Some(registration.descriptor().id.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn installed_agents(&self, repo_root: &Path) -> Vec<String> {
        self.ordered_ids
            .iter()
            .filter_map(|id| {
                let registration = self.registrations.get(id)?;
                if registration.are_hooks_installed(repo_root) {
                    Some(registration.descriptor().id.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn install_agent_hooks(
        &self,
        repo_root: &Path,
        value: &str,
        local_dev: bool,
        force: bool,
    ) -> Result<(&'static str, usize)> {
        let registration = self.resolve(value)?;
        let count = registration.install_hooks(repo_root, local_dev, force)?;
        Ok((registration.descriptor().display_name, count))
    }

    pub fn uninstall_agent_hooks(&self, repo_root: &Path, value: &str) -> Result<&'static str> {
        let registration = self.resolve(value)?;
        registration.uninstall_hooks(repo_root)?;
        Ok(registration.descriptor().display_name)
    }

    pub fn are_agent_hooks_installed(&self, repo_root: &Path, value: &str) -> Result<bool> {
        let registration = self.resolve(value)?;
        Ok(registration.are_hooks_installed(repo_root))
    }

    pub fn format_resume_command(&self, value: &str, session_id: &str) -> Result<String> {
        let registration = self.resolve(value)?;
        Ok(registration.format_resume_command(session_id))
    }

    pub fn create_all_agents(&self) -> Vec<Box<dyn super::super::Agent + Send + Sync>> {
        self.ordered_ids
            .iter()
            .map(|id| {
                self.registrations
                    .get(id)
                    .expect("adapter id missing from registry")
                    .create_agent()
            })
            .collect()
    }

    pub fn all_protected_dirs(&self) -> Vec<String> {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        for agent in self.create_all_agents() {
            for dir in agent.protected_dirs() {
                if seen.insert(dir.clone()) {
                    dirs.push(dir);
                }
            }
        }

        dirs.sort();
        dirs
    }

    pub fn validate_configuration(
        &self,
        configuration: &AgentAdapterConfiguration,
    ) -> Vec<super::types::AgentConfigValidationIssue> {
        let mut issues = Vec::new();

        let mut seen_families = HashSet::new();
        let mut seen_profiles = HashSet::new();

        for id in &self.ordered_ids {
            let registration = self
                .registrations
                .get(id)
                .expect("adapter id missing from registry");
            let descriptor = registration.descriptor();

            let family_id = descriptor.protocol_family.id.trim().to_ascii_lowercase();
            if seen_families.insert(family_id.clone()) {
                let values = configuration.families.get(&family_id);
                issues.extend(descriptor.protocol_family.config_schema.validate_values(
                    "protocol_family",
                    descriptor.protocol_family.id,
                    values,
                ));
            }

            let profile_id = descriptor.target_profile.id.trim().to_ascii_lowercase();
            if seen_profiles.insert(profile_id.clone()) {
                let values = configuration.profiles.get(&profile_id);
                issues.extend(descriptor.target_profile.config_schema.validate_values(
                    "target_profile",
                    descriptor.target_profile.id,
                    values,
                ));
            }

            let adapter_id = descriptor.id.trim().to_ascii_lowercase();
            let values = configuration.adapters.get(&adapter_id);
            issues.extend(descriptor.config_schema.validate_values(
                "adapter",
                descriptor.id,
                values,
            ));
        }

        issues
    }

    pub fn collect_readiness(&self, repo_root: &Path) -> Vec<AgentAdapterReadiness> {
        self.collect_readiness_with_config(repo_root, &AgentAdapterConfiguration::default())
    }

    pub fn collect_readiness_with_config(
        &self,
        repo_root: &Path,
        configuration: &AgentAdapterConfiguration,
    ) -> Vec<AgentAdapterReadiness> {
        self.ordered_ids
            .iter()
            .map(|id| {
                let registration = self
                    .registrations
                    .get(id)
                    .expect("adapter id missing from registry");
                let descriptor = registration.descriptor();

                let compatibility_result = self.validate_resolution_compatibility(registration);
                let compatibility_ok = compatibility_result.is_ok();

                let adapter_id = descriptor.id.trim().to_ascii_lowercase();
                let profile_id = descriptor.target_profile.id.trim().to_ascii_lowercase();
                let family_id = descriptor.protocol_family.id.trim().to_ascii_lowercase();

                let mut config_issues = descriptor.config_schema.validate_values(
                    "adapter",
                    descriptor.id,
                    configuration.adapters.get(&adapter_id),
                );
                config_issues.extend(descriptor.target_profile.config_schema.validate_values(
                    "target_profile",
                    descriptor.target_profile.id,
                    configuration.profiles.get(&profile_id),
                ));
                config_issues.extend(descriptor.protocol_family.config_schema.validate_values(
                    "protocol_family",
                    descriptor.protocol_family.id,
                    configuration.families.get(&family_id),
                ));

                let config_valid = config_issues.is_empty();
                let project_detected = registration.is_project_detected(repo_root);
                let hooks_installed = registration.are_hooks_installed(repo_root);

                let mut failures = Vec::new();
                if let Err(err) = compatibility_result {
                    failures.push(AgentReadinessFailure {
                        code: "compatibility_check_failed".to_string(),
                        message: err.to_string(),
                    });
                }
                for issue in config_issues {
                    failures.push(AgentReadinessFailure {
                        code: issue.code,
                        message: issue.message,
                    });
                }
                if !project_detected {
                    failures.push(AgentReadinessFailure {
                        code: "project_not_detected".to_string(),
                        message: "project-level agent prerequisites were not detected".to_string(),
                    });
                }

                let status = AgentReadinessStatus::from_failures(!failures.is_empty());

                AgentAdapterReadiness {
                    id: descriptor.id.to_string(),
                    display_name: descriptor.display_name.to_string(),
                    package_id: descriptor.package.id.to_string(),
                    package_metadata_version: descriptor.package.metadata_version.value(),
                    package_version: descriptor.package.version.to_string(),
                    package_source: descriptor.package.source.as_str().to_string(),
                    package_trust_model: descriptor.package.trust_model.as_str().to_string(),
                    protocol_family: descriptor.protocol_family.id.to_string(),
                    target_profile: descriptor.target_profile.id.to_string(),
                    runtime: AgentAdapterRuntime::LocalCli.as_str().to_string(),
                    project_detected,
                    hooks_installed,
                    compatibility_ok,
                    config_valid,
                    status,
                    failures,
                }
            })
            .collect()
    }

    pub fn registration_observability(&self) -> Vec<AgentRegistrationObservation> {
        self.ordered_ids
            .iter()
            .filter_map(|id| self.registrations.get(id))
            .map(|registration| {
                let descriptor = registration.descriptor();
                AgentRegistrationObservation {
                    id: descriptor.id.to_string(),
                    adapter_id: descriptor.id.to_string(),
                    package_id: descriptor.package.id.to_string(),
                    package_metadata_version: descriptor.package.metadata_version.value(),
                    package_version: descriptor.package.version.to_string(),
                    package_source: descriptor.package.source.as_str().to_string(),
                    package_trust_model: descriptor.package.trust_model.as_str().to_string(),
                    protocol_family: descriptor.protocol_family.id.to_string(),
                    target_profile: descriptor.target_profile.id.to_string(),
                    runtime: AgentAdapterRuntime::LocalCli.as_str().to_string(),
                    is_default: descriptor.is_default,
                    capabilities: descriptor
                        .capabilities
                        .iter()
                        .map(|capability| capability.as_str().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    fn validate_resolution_compatibility(
        &self,
        registration: &AgentAdapterRegistration,
    ) -> Result<()> {
        let descriptor = registration.descriptor();

        descriptor
            .package
            .validate("package", descriptor.package.id)?;
        descriptor
            .compatibility
            .validate(descriptor.id, "adapter")?;
        descriptor.runtime.validate(descriptor.id, "adapter")?;
        descriptor
            .package
            .compatibility
            .validate("package", descriptor.package.id)?;
        descriptor
            .protocol_family
            .compatibility
            .validate(descriptor.protocol_family.id, "protocol family")?;
        descriptor
            .protocol_family
            .runtime
            .validate(descriptor.protocol_family.id, "protocol family")?;
        descriptor
            .target_profile
            .compatibility
            .validate(descriptor.target_profile.id, "target profile")?;
        descriptor
            .target_profile
            .runtime
            .validate(descriptor.target_profile.id, "target profile")?;

        Ok(())
    }
}

fn validate_family_descriptor(
    existing: &AgentProtocolFamilyDescriptor,
    incoming: &AgentProtocolFamilyDescriptor,
    id: &str,
) -> Result<()> {
    if existing.display_name != incoming.display_name
        || existing.capabilities != incoming.capabilities
        || existing.compatibility != incoming.compatibility
        || existing.runtime != incoming.runtime
        || existing.config_schema != incoming.config_schema
    {
        bail!("conflicting protocol family descriptor for {id}");
    }
    Ok(())
}

fn validate_profile_descriptor(
    existing: &AgentTargetProfileDescriptor,
    incoming: &AgentTargetProfileDescriptor,
    id: &str,
) -> Result<()> {
    if existing.display_name != incoming.display_name
        || existing.family_id != incoming.family_id
        || existing.aliases != incoming.aliases
        || existing.capabilities != incoming.capabilities
        || existing.compatibility != incoming.compatibility
        || existing.runtime != incoming.runtime
        || existing.config_schema != incoming.config_schema
    {
        bail!("conflicting target profile descriptor for {id}");
    }
    Ok(())
}

fn register_alias(
    aliases: &mut HashMap<String, String>,
    sources: &mut HashMap<String, AliasResolutionSource>,
    id: &str,
    alias: &str,
    source: AliasResolutionSource,
) -> Result<()> {
    let alias_key = normalise_key(alias)?;
    if let Some(existing) = aliases.get(&alias_key)
        && existing != id
    {
        bail!("alias collision for {alias_key}: {existing} vs {id}");
    }

    aliases.insert(alias_key.clone(), id.to_string());
    sources.insert(alias_key, source);
    Ok(())
}

fn register_profile_alias(
    aliases: &mut HashMap<String, String>,
    profile_id: &str,
    alias: &str,
) -> Result<()> {
    let alias_key = normalise_key(alias)?;
    if let Some(existing) = aliases.get(&alias_key)
        && existing != profile_id
    {
        bail!("target profile alias collision for {alias_key}: {existing} vs {profile_id}");
    }
    aliases.insert(alias_key, profile_id.to_string());
    Ok(())
}
