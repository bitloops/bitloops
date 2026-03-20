use std::collections::{HashMap, HashSet};

use semver::{Version, VersionReq};

use super::descriptors::{
    LanguagePackDescriptor, LanguagePackRegistrationObservation, LanguagePackRegistrationStatus,
};
use super::errors::{LanguagePackRegistryError, LanguagePackResolutionError};
use super::normalise::{
    extract_normalised_extension, normalise_extension, normalise_identifier,
    normalise_resolution_identifier, parse_source_version,
};
use super::resolution::{
    LanguagePackResolutionInput, LanguageProfile, LanguageResolutionSource, ResolvedLanguagePack,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileOwner {
    pack_id: String,
    profile_id: String,
    language_id: String,
    dialect: Option<String>,
    source_version_requirements: Vec<VersionReq>,
    extensions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LanguagePackRegistry {
    descriptors: HashMap<String, LanguagePackDescriptor>,
    aliases: HashMap<String, String>,
    language_owners: HashMap<String, String>,
    profile_aliases: HashMap<String, String>,
    profiles: HashMap<String, ProfileOwner>,
    profiles_by_language: HashMap<String, Vec<String>>,
    profiles_by_extension: HashMap<String, Vec<String>>,
    observations: Vec<LanguagePackRegistrationObservation>,
}

impl LanguagePackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        descriptor: LanguagePackDescriptor,
    ) -> Result<(), LanguagePackRegistryError> {
        let pack_id = normalise_identifier(descriptor.id, "language pack id")?;
        if semver::Version::parse(descriptor.version).is_err() {
            let error = LanguagePackRegistryError::InvalidPackVersion {
                pack_id: pack_id.clone(),
                version: descriptor.version.to_string(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        if descriptor.supported_languages.is_empty() {
            let error = LanguagePackRegistryError::MissingSupportedLanguages {
                pack_id: pack_id.clone(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        if descriptor.language_profiles.is_empty() {
            let error = LanguagePackRegistryError::MissingLanguageProfiles {
                pack_id: pack_id.clone(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        if self.descriptors.contains_key(&pack_id) {
            let error = LanguagePackRegistryError::DuplicatePackId {
                pack_id: pack_id.clone(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        let mut normalised_aliases = Vec::new();
        for alias in descriptor.aliases {
            let normalised_alias = normalise_identifier(alias, "language pack alias")?;
            if let Some(existing_pack_id) = self.aliases.get(&normalised_alias)
                && existing_pack_id != &pack_id
            {
                let error = LanguagePackRegistryError::AliasConflict {
                    alias: normalised_alias,
                    existing_pack_id: existing_pack_id.clone(),
                    attempted_pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
            normalised_aliases.push(normalised_alias);
        }

        let mut unique_languages = HashSet::new();
        let mut normalised_languages = Vec::new();
        for language in descriptor.supported_languages {
            let normalised_language = normalise_identifier(language, "language identifier")?;
            if !unique_languages.insert(normalised_language.clone()) {
                continue;
            }
            if let Some(existing_pack_id) = self.language_owners.get(&normalised_language)
                && existing_pack_id != &pack_id
            {
                let error = LanguagePackRegistryError::LanguageAlreadyOwned {
                    language: normalised_language,
                    existing_pack_id: existing_pack_id.clone(),
                    attempted_pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
            normalised_languages.push(normalised_language);
        }

        let mut profile_alias_bindings = Vec::new();
        let mut profile_owners = Vec::new();
        let mut seen_profile_ids = HashSet::new();

        for profile in descriptor.language_profiles {
            let normalised_profile_id = normalise_identifier(profile.id, "language profile id")?;
            if !seen_profile_ids.insert(normalised_profile_id.clone()) {
                let error = LanguagePackRegistryError::DuplicateProfileId {
                    profile_id: normalised_profile_id,
                    pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }

            let normalised_profile_language =
                normalise_identifier(profile.language_id, "language profile language id")?;
            if !normalised_languages.contains(&normalised_profile_language) {
                let error = LanguagePackRegistryError::ProfileLanguageNotSupported {
                    profile_id: normalised_profile_id.clone(),
                    language: normalised_profile_language,
                    pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
            let normalised_profile_dialect = profile
                .dialect
                .map(|dialect| normalise_identifier(dialect, "language profile dialect"))
                .transpose()?;

            let profile_lookup_key = format!("{pack_id}::{normalised_profile_id}");
            let mut profile_aliases = Vec::new();
            profile_aliases.push(normalised_profile_id.clone());
            for alias in profile.aliases {
                profile_aliases.push(normalise_identifier(alias, "language profile alias")?);
            }
            for alias in profile_aliases {
                if let Some(existing_profile_key) = self.profile_aliases.get(&alias)
                    && existing_profile_key != &profile_lookup_key
                {
                    let error = LanguagePackRegistryError::ProfileAliasConflict {
                        alias,
                        existing_profile_key: existing_profile_key.clone(),
                        attempted_profile_key: profile_lookup_key.clone(),
                    };
                    self.push_rejection(&pack_id, error.to_string());
                    return Err(error);
                }
                profile_alias_bindings.push((alias, profile_lookup_key.clone()));
            }

            let mut normalised_extensions = Vec::new();
            let mut seen_extensions = HashSet::new();
            for extension in profile.file_extensions {
                let Some(normalised_extension) =
                    normalise_extension(extension, "language profile file extension")?
                else {
                    continue;
                };
                if seen_extensions.insert(normalised_extension.clone()) {
                    normalised_extensions.push(normalised_extension);
                }
            }
            let mut source_version_requirements = Vec::new();
            for requirement in profile.supported_source_versions {
                let trimmed_requirement = requirement.trim();
                if trimmed_requirement.is_empty() {
                    continue;
                }
                let parsed_requirement = VersionReq::parse(trimmed_requirement).map_err(|_| {
                    let error = LanguagePackRegistryError::InvalidProfileSourceVersionRequirement {
                        pack_id: pack_id.clone(),
                        profile_id: normalised_profile_id.clone(),
                        requirement: trimmed_requirement.to_string(),
                    };
                    self.push_rejection(&pack_id, error.to_string());
                    error
                })?;
                source_version_requirements.push(parsed_requirement);
            }

            profile_owners.push((
                profile_lookup_key,
                ProfileOwner {
                    pack_id: pack_id.clone(),
                    profile_id: normalised_profile_id,
                    language_id: normalised_profile_language,
                    dialect: normalised_profile_dialect,
                    source_version_requirements,
                    extensions: normalised_extensions,
                },
            ));
        }

        self.aliases.insert(pack_id.clone(), pack_id.clone());
        for alias in normalised_aliases {
            self.aliases.insert(alias, pack_id.clone());
        }
        for language in normalised_languages {
            self.language_owners.insert(language, pack_id.clone());
        }
        for (alias, profile_lookup_key) in profile_alias_bindings {
            self.profile_aliases.insert(alias, profile_lookup_key);
        }
        for (profile_lookup_key, owner) in profile_owners {
            for extension in &owner.extensions {
                self.profiles_by_extension
                    .entry(extension.clone())
                    .or_default()
                    .push(profile_lookup_key.clone());
            }
            self.profiles_by_language
                .entry(owner.language_id.clone())
                .or_default()
                .push(profile_lookup_key.clone());
            self.profiles.insert(profile_lookup_key, owner);
        }
        self.descriptors.insert(pack_id.clone(), descriptor);
        self.observations.push(LanguagePackRegistrationObservation {
            pack_id,
            status: LanguagePackRegistrationStatus::Registered,
            reason: None,
        });

        Ok(())
    }

    pub fn resolve_pack(&self, pack_key: &str) -> Option<&LanguagePackDescriptor> {
        let normalised_key = normalise_identifier(pack_key, "language pack key").ok()?;
        let resolved_pack_id = self.aliases.get(&normalised_key)?;
        self.descriptors.get(resolved_pack_id)
    }

    pub fn resolve_for_language(&self, language: &str) -> Option<&LanguagePackDescriptor> {
        let normalised_language = normalise_identifier(language, "language identifier").ok()?;
        let pack_id = self.language_owners.get(&normalised_language)?;
        self.descriptors.get(pack_id)
    }

    pub fn resolve_profile(&self, profile_key: &str) -> Option<ResolvedLanguagePack<'_>> {
        let normalised_profile_key =
            normalise_identifier(profile_key, "language profile lookup key").ok()?;
        let profile_lookup_key = self.profile_aliases.get(&normalised_profile_key)?;
        self.resolved_from_profile_lookup(profile_lookup_key, LanguageResolutionSource::ProfileKey)
    }

    pub fn resolve(
        &self,
        input: LanguagePackResolutionInput<'_>,
    ) -> Result<ResolvedLanguagePack<'_>, LanguagePackResolutionError> {
        if let Some(profile_key) = input.profile_key {
            let resolved = self.resolve_profile(profile_key).ok_or_else(|| {
                LanguagePackResolutionError::UnsupportedProfile {
                    profile_key: profile_key.trim().to_string(),
                }
            })?;
            if let Some(language_id) = input.language_id {
                let normalised_language = normalise_resolution_identifier(language_id);
                if resolved.profile.language_id.to_ascii_lowercase() != normalised_language {
                    return Err(LanguagePackResolutionError::ProfileLanguageMismatch {
                        profile_key: profile_key.trim().to_string(),
                        language: normalised_language,
                    });
                }
            }
            return Ok(resolved);
        }

        if let Some(language_profile) = input.language_profile {
            return self.resolve_for_language_profile(language_profile, input.file_path);
        }

        if let Some(language_id) = input.language_id {
            let normalised_language = normalise_resolution_identifier(language_id);
            let candidates = self
                .profiles_by_language
                .get(&normalised_language)
                .cloned()
                .unwrap_or_default();
            if candidates.is_empty() {
                return Err(LanguagePackResolutionError::UnsupportedLanguage {
                    language: normalised_language,
                });
            }

            if let Some(file_path) = input.file_path
                && let Some(extension) = extract_normalised_extension(file_path)
            {
                let extension_matches = candidates
                    .iter()
                    .filter(|candidate| self.profile_matches_extension(candidate, &extension))
                    .cloned()
                    .collect::<Vec<_>>();
                if extension_matches.len() == 1 {
                    return self
                        .resolved_from_profile_lookup(
                            &extension_matches[0],
                            LanguageResolutionSource::LanguageIdAndFilePath,
                        )
                        .ok_or_else(|| LanguagePackResolutionError::UnsupportedLanguage {
                            language: normalised_language.clone(),
                        });
                }
                if extension_matches.len() > 1 {
                    return Err(LanguagePackResolutionError::AmbiguousFileExtension {
                        extension,
                        profile_ids: self.profile_ids_for_lookup_keys(&extension_matches),
                    });
                }
            }

            if candidates.len() == 1 {
                return self
                    .resolved_from_profile_lookup(
                        &candidates[0],
                        LanguageResolutionSource::LanguageId,
                    )
                    .ok_or(LanguagePackResolutionError::UnsupportedLanguage {
                        language: normalised_language,
                    });
            }

            return Err(LanguagePackResolutionError::AmbiguousLanguageProfiles {
                language: normalised_language,
                profile_ids: self.profile_ids_for_lookup_keys(&candidates),
            });
        }

        if let Some(file_path) = input.file_path {
            let extension = extract_normalised_extension(file_path).ok_or_else(|| {
                LanguagePackResolutionError::UnsupportedFileExtension {
                    extension: String::new(),
                }
            })?;
            let candidates = self
                .profiles_by_extension
                .get(&extension)
                .cloned()
                .unwrap_or_default();
            if candidates.is_empty() {
                return Err(LanguagePackResolutionError::UnsupportedFileExtension { extension });
            }
            if candidates.len() > 1 {
                return Err(LanguagePackResolutionError::AmbiguousFileExtension {
                    extension,
                    profile_ids: self.profile_ids_for_lookup_keys(&candidates),
                });
            }
            return self
                .resolved_from_profile_lookup(&candidates[0], LanguageResolutionSource::FilePath)
                .ok_or(LanguagePackResolutionError::MissingResolutionInput);
        }

        Err(LanguagePackResolutionError::MissingResolutionInput)
    }

    pub fn owner_for_language(&self, language: &str) -> Option<&str> {
        let normalised_language = normalise_identifier(language, "language identifier").ok()?;
        self.language_owners
            .get(&normalised_language)
            .map(String::as_str)
    }

    pub fn observations(&self) -> &[LanguagePackRegistrationObservation] {
        &self.observations
    }

    pub fn registered_pack_ids(&self) -> Vec<&str> {
        let mut ids = self
            .descriptors
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    fn profile_matches_extension(&self, lookup_key: &str, extension: &str) -> bool {
        self.profiles.get(lookup_key).is_some_and(|owner| {
            owner
                .extensions
                .iter()
                .any(|candidate| candidate == extension)
        })
    }

    fn profile_matches_dialect(&self, lookup_key: &str, dialect: &str) -> bool {
        self.profiles.get(lookup_key).is_some_and(|owner| {
            owner
                .dialect
                .as_ref()
                .is_some_and(|candidate| candidate == dialect)
        })
    }

    fn profile_matches_source_version(&self, lookup_key: &str, source_version: &Version) -> bool {
        self.profiles.get(lookup_key).is_some_and(|owner| {
            owner.source_version_requirements.is_empty()
                || owner
                    .source_version_requirements
                    .iter()
                    .any(|requirement| requirement.matches(source_version))
        })
    }

    fn resolve_for_language_profile(
        &self,
        language_profile: LanguageProfile<'_>,
        file_path: Option<&str>,
    ) -> Result<ResolvedLanguagePack<'_>, LanguagePackResolutionError> {
        let normalised_language = normalise_resolution_identifier(language_profile.language_id);
        let mut candidates = self
            .profiles_by_language
            .get(&normalised_language)
            .cloned()
            .unwrap_or_default();
        if candidates.is_empty() {
            return Err(LanguagePackResolutionError::UnsupportedLanguage {
                language: normalised_language,
            });
        }

        if let Some(dialect) = language_profile.dialect {
            let normalised_dialect = normalise_resolution_identifier(dialect);
            let exact_dialect_matches = candidates
                .iter()
                .filter(|candidate| self.profile_matches_dialect(candidate, &normalised_dialect))
                .cloned()
                .collect::<Vec<_>>();
            if exact_dialect_matches.is_empty() {
                return Err(LanguagePackResolutionError::UnsupportedDialect {
                    language: normalised_language,
                    dialect: normalised_dialect,
                });
            }
            candidates = exact_dialect_matches;
        }

        if let Some(source_version) = language_profile.source_version {
            let normalised_source_version = source_version.trim().to_string();
            let parsed_source_version = parse_source_version(source_version).ok_or_else(|| {
                LanguagePackResolutionError::InvalidSourceVersion {
                    source_version: normalised_source_version.clone(),
                }
            })?;
            let matching_versions = candidates
                .iter()
                .filter(|candidate| {
                    self.profile_matches_source_version(candidate, &parsed_source_version)
                })
                .cloned()
                .collect::<Vec<_>>();
            if matching_versions.is_empty() {
                return Err(LanguagePackResolutionError::UnsupportedSourceVersion {
                    language: normalised_language,
                    source_version: normalised_source_version,
                });
            }
            candidates = matching_versions;
        }

        if let Some(path) = file_path
            && let Some(extension) = extract_normalised_extension(path)
        {
            let extension_matches = candidates
                .iter()
                .filter(|candidate| self.profile_matches_extension(candidate, &extension))
                .cloned()
                .collect::<Vec<_>>();
            if extension_matches.is_empty() {
                return Err(LanguagePackResolutionError::UnsupportedFileExtension { extension });
            }
            if extension_matches.len() > 1 {
                return Err(LanguagePackResolutionError::AmbiguousFileExtension {
                    extension,
                    profile_ids: self.profile_ids_for_lookup_keys(&extension_matches),
                });
            }
            return self
                .resolved_from_profile_lookup(
                    &extension_matches[0],
                    LanguageResolutionSource::LanguageProfileAndFilePath,
                )
                .ok_or_else(|| LanguagePackResolutionError::UnsupportedLanguage {
                    language: normalised_language.clone(),
                });
        }

        if candidates.len() == 1 {
            return self
                .resolved_from_profile_lookup(
                    &candidates[0],
                    LanguageResolutionSource::LanguageProfile,
                )
                .ok_or(LanguagePackResolutionError::UnsupportedLanguage {
                    language: normalised_language,
                });
        }

        Err(LanguagePackResolutionError::AmbiguousLanguageProfiles {
            language: normalised_language,
            profile_ids: self.profile_ids_for_lookup_keys(&candidates),
        })
    }

    fn profile_ids_for_lookup_keys(&self, lookup_keys: &[String]) -> Vec<String> {
        let mut profile_ids = lookup_keys
            .iter()
            .filter_map(|lookup_key| self.profiles.get(lookup_key))
            .map(|owner| format!("{}:{}", owner.pack_id, owner.profile_id))
            .collect::<Vec<_>>();
        profile_ids.sort();
        profile_ids
    }

    fn resolved_from_profile_lookup(
        &self,
        lookup_key: &str,
        source: LanguageResolutionSource,
    ) -> Option<ResolvedLanguagePack<'_>> {
        let owner = self.profiles.get(lookup_key)?;
        let pack = self.descriptors.get(&owner.pack_id)?;
        let profile = pack
            .language_profiles
            .iter()
            .find(|profile| normalise_resolution_identifier(profile.id) == owner.profile_id)?;
        Some(ResolvedLanguagePack {
            pack,
            profile,
            source,
        })
    }

    fn push_rejection(&mut self, pack_id: &str, reason: String) {
        self.observations.push(LanguagePackRegistrationObservation {
            pack_id: pack_id.to_string(),
            status: LanguagePackRegistrationStatus::Rejected,
            reason: Some(reason),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::extensions::lifecycle::ExtensionCompatibility;

    use super::super::{
        LanguagePackDescriptor, LanguagePackRegistry, LanguagePackRegistryError,
        LanguagePackResolutionError, LanguagePackResolutionInput, LanguageProfile,
        LanguageProfileDescriptor, LanguageResolutionSource,
    };

    const RUST_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
        id: "rust-pack",
        version: "1.0.0",
        api_version: 1,
        display_name: "Rust",
        aliases: &["rust"],
        supported_languages: &["rust"],
        language_profiles: &[LanguageProfileDescriptor {
            id: "rust-default",
            display_name: "Rust Default",
            language_id: "rust",
            dialect: None,
            aliases: &["rust-profile"],
            file_extensions: &["rs"],
            supported_source_versions: &["^1.70"],
        }],
        compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
    };

    const TS_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
        id: "ts-pack",
        version: "1.0.0",
        api_version: 1,
        display_name: "TypeScript/JavaScript",
        aliases: &["typescript-pack"],
        supported_languages: &["typescript", "javascript"],
        language_profiles: &[
            LanguageProfileDescriptor {
                id: "typescript-standard",
                display_name: "TypeScript Standard",
                language_id: "typescript",
                dialect: Some("ts"),
                aliases: &["ts"],
                file_extensions: &["ts", "tsx"],
                supported_source_versions: &["^5.0"],
            },
            LanguageProfileDescriptor {
                id: "javascript-standard",
                display_name: "JavaScript Standard",
                language_id: "javascript",
                dialect: Some("js"),
                aliases: &["js"],
                file_extensions: &["js", "jsx"],
                supported_source_versions: &[],
            },
        ],
        compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
    };

    #[test]
    fn language_pack_registry_registers_and_resolves_by_language_alias_and_profile() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(RUST_PACK).expect("register rust pack");
        registry.register(TS_PACK).expect("register ts pack");

        let rust_owner = registry
            .resolve_for_language("rust")
            .expect("resolve rust owner");
        assert_eq!(rust_owner.id, "rust-pack");

        let alias_owner = registry
            .resolve_pack("typescript-pack")
            .expect("resolve alias owner");
        assert_eq!(alias_owner.id, "ts-pack");

        let profile_resolution = registry
            .resolve(LanguagePackResolutionInput::for_profile("js"))
            .expect("resolve profile");
        assert_eq!(profile_resolution.pack.id, "ts-pack");
        assert_eq!(profile_resolution.profile.id, "javascript-standard");
        assert_eq!(
            profile_resolution.source,
            LanguageResolutionSource::ProfileKey
        );
    }

    #[test]
    fn language_pack_registry_resolves_by_file_path_and_language_context() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(RUST_PACK).expect("register rust pack");
        registry.register(TS_PACK).expect("register ts pack");

        let by_file = registry
            .resolve(LanguagePackResolutionInput::for_file_path("src/main.ts"))
            .expect("resolve by path");
        assert_eq!(by_file.pack.id, "ts-pack");
        assert_eq!(by_file.profile.id, "typescript-standard");
        assert_eq!(by_file.source, LanguageResolutionSource::FilePath);

        let by_language_and_path = registry
            .resolve(
                LanguagePackResolutionInput::for_language("typescript")
                    .with_file_path("src/main.tsx"),
            )
            .expect("resolve by language and path");
        assert_eq!(by_language_and_path.pack.id, "ts-pack");
        assert_eq!(by_language_and_path.profile.id, "typescript-standard");
        assert_eq!(
            by_language_and_path.source,
            LanguageResolutionSource::LanguageIdAndFilePath
        );
    }

    #[test]
    fn language_pack_registry_rejects_duplicate_pack_ids() {
        let mut registry = LanguagePackRegistry::new();
        registry
            .register(RUST_PACK)
            .expect("register initial rust pack");

        let error = registry
            .register(LanguagePackDescriptor {
                id: "rust-pack",
                version: "1.0.1",
                api_version: 1,
                display_name: "Rust duplicate",
                aliases: &[],
                supported_languages: &["rust-alt"],
                language_profiles: &[LanguageProfileDescriptor {
                    id: "rust-alt-default",
                    display_name: "Rust Alt",
                    language_id: "rust-alt",
                    dialect: None,
                    aliases: &[],
                    file_extensions: &["rs"],
                    supported_source_versions: &[],
                }],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect_err("duplicate pack id should fail");

        assert!(matches!(
            error,
            LanguagePackRegistryError::DuplicatePackId { .. }
        ));
    }

    #[test]
    fn language_pack_registry_rejects_language_collisions() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(RUST_PACK).expect("register rust pack");

        let error = registry
            .register(LanguagePackDescriptor {
                id: "another-rust",
                version: "1.0.0",
                api_version: 1,
                display_name: "Another Rust",
                aliases: &[],
                supported_languages: &["rust"],
                language_profiles: &[LanguageProfileDescriptor {
                    id: "rust-alt-profile",
                    display_name: "Rust Alt",
                    language_id: "rust",
                    dialect: None,
                    aliases: &[],
                    file_extensions: &["rs"],
                    supported_source_versions: &[],
                }],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect_err("language ownership collision should fail");

        assert!(matches!(
            error,
            LanguagePackRegistryError::LanguageAlreadyOwned { .. }
        ));
    }

    #[test]
    fn language_pack_registry_rejects_profile_alias_collisions() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(TS_PACK).expect("register ts pack");

        let error = registry
            .register(LanguagePackDescriptor {
                id: "alt-ts-pack",
                version: "1.0.0",
                api_version: 1,
                display_name: "Alt TypeScript",
                aliases: &[],
                supported_languages: &["typescript-alt"],
                language_profiles: &[LanguageProfileDescriptor {
                    id: "typescript-alt-profile",
                    display_name: "TS Alt",
                    language_id: "typescript-alt",
                    dialect: Some("ts"),
                    aliases: &["js"],
                    file_extensions: &["mts"],
                    supported_source_versions: &[],
                }],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect_err("profile alias collision should fail");

        assert!(matches!(
            error,
            LanguagePackRegistryError::ProfileAliasConflict { .. }
        ));
    }

    #[test]
    fn language_pack_registry_reports_ambiguous_profile_resolution() {
        let mut registry = LanguagePackRegistry::new();
        registry
            .register(LanguagePackDescriptor {
                id: "ambiguous-ts-pack",
                version: "1.0.0",
                api_version: 1,
                display_name: "Ambiguous TypeScript",
                aliases: &[],
                supported_languages: &["typescript"],
                language_profiles: &[
                    LanguageProfileDescriptor {
                        id: "ts-web",
                        display_name: "TS Web",
                        language_id: "typescript",
                        dialect: Some("ts"),
                        aliases: &[],
                        file_extensions: &["tsx"],
                        supported_source_versions: &[],
                    },
                    LanguageProfileDescriptor {
                        id: "ts-node",
                        display_name: "TS Node",
                        language_id: "typescript",
                        dialect: Some("ts"),
                        aliases: &[],
                        file_extensions: &["mts"],
                        supported_source_versions: &[],
                    },
                ],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect("register ambiguous profile pack");

        let error = registry
            .resolve(LanguagePackResolutionInput::for_language("typescript"))
            .expect_err("language-only resolution should be ambiguous");
        assert!(matches!(
            error,
            LanguagePackResolutionError::AmbiguousLanguageProfiles { .. }
        ));
    }

    #[test]
    fn language_pack_registry_reports_unsupported_resolution_cases() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(RUST_PACK).expect("register rust pack");

        let missing_language = registry
            .resolve(LanguagePackResolutionInput::for_language("python"))
            .expect_err("unsupported language should fail");
        assert!(matches!(
            missing_language,
            LanguagePackResolutionError::UnsupportedLanguage { .. }
        ));

        let missing_extension = registry
            .resolve(LanguagePackResolutionInput::for_file_path("src/main.py"))
            .expect_err("unsupported extension should fail");
        assert!(matches!(
            missing_extension,
            LanguagePackResolutionError::UnsupportedFileExtension { .. }
        ));
    }

    #[test]
    fn language_pack_registry_resolves_language_profile_with_dialect_and_source_version() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(TS_PACK).expect("register ts pack");

        let resolved = registry
            .resolve(
                LanguagePackResolutionInput::for_language_profile(
                    LanguageProfile::new("typescript")
                        .with_dialect("ts")
                        .with_source_version("5.3"),
                )
                .with_file_path("src/main.ts"),
            )
            .expect("profile-aware resolution should succeed");

        assert_eq!(resolved.pack.id, "ts-pack");
        assert_eq!(resolved.profile.id, "typescript-standard");
        assert_eq!(
            resolved.source,
            LanguageResolutionSource::LanguageProfileAndFilePath
        );
    }

    #[test]
    fn language_pack_registry_rejects_unsupported_profile_source_version() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(TS_PACK).expect("register ts pack");

        let error = registry
            .resolve(LanguagePackResolutionInput::for_language_profile(
                LanguageProfile::new("typescript")
                    .with_dialect("ts")
                    .with_source_version("4.3"),
            ))
            .expect_err("unsupported source version should fail");
        assert!(matches!(
            error,
            LanguagePackResolutionError::UnsupportedSourceVersion { .. }
        ));
    }

    #[test]
    fn language_pack_registry_rejects_unsupported_profile_dialect() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(TS_PACK).expect("register ts pack");

        let error = registry
            .resolve(LanguagePackResolutionInput::for_language_profile(
                LanguageProfile::new("typescript").with_dialect("tsx"),
            ))
            .expect_err("unsupported dialect should fail");
        assert!(matches!(
            error,
            LanguagePackResolutionError::UnsupportedDialect { .. }
        ));
    }

    #[test]
    fn language_pack_registry_rejects_invalid_pack_and_profile_version_metadata() {
        let mut registry = LanguagePackRegistry::new();
        let pack_version_error = registry
            .register(LanguagePackDescriptor {
                id: "invalid-pack-version",
                version: "not-a-version",
                api_version: 1,
                display_name: "Invalid Pack Version",
                aliases: &[],
                supported_languages: &["typescript"],
                language_profiles: &[LanguageProfileDescriptor {
                    id: "invalid-profile",
                    display_name: "Invalid Profile",
                    language_id: "typescript",
                    dialect: Some("ts"),
                    aliases: &[],
                    file_extensions: &["ts"],
                    supported_source_versions: &["^5.0"],
                }],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect_err("invalid pack version should fail registration");
        assert!(matches!(
            pack_version_error,
            LanguagePackRegistryError::InvalidPackVersion { .. }
        ));

        let mut registry = LanguagePackRegistry::new();
        let requirement_error = registry
            .register(LanguagePackDescriptor {
                id: "invalid-profile-requirement",
                version: "1.0.0",
                api_version: 1,
                display_name: "Invalid Profile Requirement",
                aliases: &[],
                supported_languages: &["typescript"],
                language_profiles: &[LanguageProfileDescriptor {
                    id: "invalid-profile-requirement",
                    display_name: "Invalid Profile Requirement",
                    language_id: "typescript",
                    dialect: Some("ts"),
                    aliases: &[],
                    file_extensions: &["ts"],
                    supported_source_versions: &["(not-valid"],
                }],
                compatibility: ExtensionCompatibility::phase1_local_cli(&["language-packs"]),
            })
            .expect_err("invalid source-version requirement should fail registration");
        assert!(matches!(
            requirement_error,
            LanguagePackRegistryError::InvalidProfileSourceVersionRequirement { .. }
        ));
    }
}
