use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub supported_languages: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguagePackRegistrationStatus {
    Registered,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackRegistrationObservation {
    pub pack_id: String,
    pub status: LanguagePackRegistrationStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguagePackRegistryError {
    InvalidIdentifier {
        field: &'static str,
        value: String,
    },
    MissingSupportedLanguages {
        pack_id: String,
    },
    DuplicatePackId {
        pack_id: String,
    },
    AliasConflict {
        alias: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
    LanguageAlreadyOwned {
        language: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
}

impl Display for LanguagePackRegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field, value } => {
                write!(f, "invalid {field}: `{value}`")
            }
            Self::MissingSupportedLanguages { pack_id } => {
                write!(
                    f,
                    "language pack `{pack_id}` must declare at least one supported language"
                )
            }
            Self::DuplicatePackId { pack_id } => {
                write!(f, "duplicate language pack id: `{pack_id}`")
            }
            Self::AliasConflict {
                alias,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "language pack alias `{alias}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
            Self::LanguageAlreadyOwned {
                language,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "language `{language}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
        }
    }
}

impl Error for LanguagePackRegistryError {}

#[derive(Debug, Clone, Default)]
pub struct LanguagePackRegistry {
    descriptors: HashMap<String, LanguagePackDescriptor>,
    aliases: HashMap<String, String>,
    language_owners: HashMap<String, String>,
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

        if descriptor.supported_languages.is_empty() {
            let error = LanguagePackRegistryError::MissingSupportedLanguages {
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

        self.aliases.insert(pack_id.clone(), pack_id.clone());
        for alias in normalised_aliases {
            self.aliases.insert(alias, pack_id.clone());
        }
        for language in normalised_languages {
            self.language_owners.insert(language, pack_id.clone());
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

    fn push_rejection(&mut self, pack_id: &str, reason: String) {
        self.observations.push(LanguagePackRegistrationObservation {
            pack_id: pack_id.to_string(),
            status: LanguagePackRegistrationStatus::Rejected,
            reason: Some(reason),
        });
    }
}

fn normalise_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, LanguagePackRegistryError> {
    let normalised = value.trim().to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(LanguagePackRegistryError::InvalidIdentifier {
            field,
            value: value.to_string(),
        });
    }
    Ok(normalised)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
        id: "rust-pack",
        display_name: "Rust",
        aliases: &["rust"],
        supported_languages: &["rust"],
    };

    const TS_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
        id: "ts-pack",
        display_name: "TypeScript/JavaScript",
        aliases: &["typescript-pack"],
        supported_languages: &["typescript", "javascript"],
    };

    #[test]
    fn language_pack_registry_registers_and_resolves_by_language_and_alias() {
        let mut registry = LanguagePackRegistry::new();
        registry.register(RUST_PACK).expect("register rust pack");
        registry.register(TS_PACK).expect("register ts pack");

        let rust_owner = registry
            .resolve_for_language("rust")
            .expect("resolve rust owner");
        assert_eq!(rust_owner.id, "rust-pack");

        let ts_owner = registry
            .resolve_for_language("JavaScript")
            .expect("resolve javascript owner");
        assert_eq!(ts_owner.id, "ts-pack");

        let alias_owner = registry
            .resolve_pack("typescript-pack")
            .expect("resolve alias owner");
        assert_eq!(alias_owner.id, "ts-pack");
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
                display_name: "Rust duplicate",
                aliases: &[],
                supported_languages: &["rust-alt"],
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
                display_name: "Another Rust",
                aliases: &[],
                supported_languages: &["rust"],
            })
            .expect_err("language ownership collision should fail");

        assert!(matches!(
            error,
            LanguagePackRegistryError::LanguageAlreadyOwned { .. }
        ));
        assert_eq!(
            registry.owner_for_language("rust"),
            Some("rust-pack"),
            "existing ownership should remain unchanged after rejection"
        );
    }
}
