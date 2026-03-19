use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPackDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub stage_contributions: &'static [&'static str],
    pub ingester_contributions: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityPackRegistrationStatus {
    Registered,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPackRegistrationObservation {
    pub pack_id: String,
    pub status: CapabilityPackRegistrationStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityPackRegistryError {
    InvalidIdentifier {
        field: &'static str,
        value: String,
    },
    MissingContributions {
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
    StageAlreadyOwned {
        stage: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
    IngesterAlreadyOwned {
        ingester: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
}

impl Display for CapabilityPackRegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field, value } => {
                write!(f, "invalid {field}: `{value}`")
            }
            Self::MissingContributions { pack_id } => {
                write!(
                    f,
                    "capability pack `{pack_id}` must declare at least one stage or ingester contribution"
                )
            }
            Self::DuplicatePackId { pack_id } => {
                write!(f, "duplicate capability pack id: `{pack_id}`")
            }
            Self::AliasConflict {
                alias,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "capability pack alias `{alias}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
            Self::StageAlreadyOwned {
                stage,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "stage `{stage}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
            Self::IngesterAlreadyOwned {
                ingester,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "ingester `{ingester}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
        }
    }
}

impl Error for CapabilityPackRegistryError {}

#[derive(Debug, Clone, Default)]
pub struct CapabilityPackRegistry {
    descriptors: HashMap<String, CapabilityPackDescriptor>,
    aliases: HashMap<String, String>,
    stage_owners: HashMap<String, String>,
    ingester_owners: HashMap<String, String>,
    observations: Vec<CapabilityPackRegistrationObservation>,
}

impl CapabilityPackRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        descriptor: CapabilityPackDescriptor,
    ) -> Result<(), CapabilityPackRegistryError> {
        let pack_id = normalise_identifier(descriptor.id, "capability pack id")?;

        if descriptor.stage_contributions.is_empty() && descriptor.ingester_contributions.is_empty()
        {
            let error = CapabilityPackRegistryError::MissingContributions {
                pack_id: pack_id.clone(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        if self.descriptors.contains_key(&pack_id) {
            let error = CapabilityPackRegistryError::DuplicatePackId {
                pack_id: pack_id.clone(),
            };
            self.push_rejection(&pack_id, error.to_string());
            return Err(error);
        }

        for alias in descriptor.aliases {
            let normalised_alias = normalise_identifier(alias, "capability pack alias")?;
            if let Some(existing_pack_id) = self.aliases.get(&normalised_alias)
                && existing_pack_id != &pack_id
            {
                let error = CapabilityPackRegistryError::AliasConflict {
                    alias: normalised_alias,
                    existing_pack_id: existing_pack_id.clone(),
                    attempted_pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
        }

        for stage in descriptor.stage_contributions {
            let normalised_stage = normalise_identifier(stage, "capability stage")?;
            if let Some(existing_pack_id) = self.stage_owners.get(&normalised_stage)
                && existing_pack_id != &pack_id
            {
                let error = CapabilityPackRegistryError::StageAlreadyOwned {
                    stage: normalised_stage,
                    existing_pack_id: existing_pack_id.clone(),
                    attempted_pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
        }

        for ingester in descriptor.ingester_contributions {
            let normalised_ingester = normalise_identifier(ingester, "capability ingester")?;
            if let Some(existing_pack_id) = self.ingester_owners.get(&normalised_ingester)
                && existing_pack_id != &pack_id
            {
                let error = CapabilityPackRegistryError::IngesterAlreadyOwned {
                    ingester: normalised_ingester,
                    existing_pack_id: existing_pack_id.clone(),
                    attempted_pack_id: pack_id.clone(),
                };
                self.push_rejection(&pack_id, error.to_string());
                return Err(error);
            }
        }

        self.aliases.insert(pack_id.clone(), pack_id.clone());
        for alias in descriptor.aliases {
            let normalised_alias = normalise_identifier(alias, "capability pack alias")?;
            self.aliases.insert(normalised_alias, pack_id.clone());
        }
        for stage in descriptor.stage_contributions {
            let normalised_stage = normalise_identifier(stage, "capability stage")?;
            self.stage_owners.insert(normalised_stage, pack_id.clone());
        }
        for ingester in descriptor.ingester_contributions {
            let normalised_ingester = normalise_identifier(ingester, "capability ingester")?;
            self.ingester_owners
                .insert(normalised_ingester, pack_id.clone());
        }
        self.descriptors.insert(pack_id.clone(), descriptor);
        self.observations
            .push(CapabilityPackRegistrationObservation {
                pack_id,
                status: CapabilityPackRegistrationStatus::Registered,
                reason: None,
            });
        Ok(())
    }

    pub fn resolve_pack(&self, pack_key: &str) -> Option<&CapabilityPackDescriptor> {
        let normalised_key = normalise_identifier(pack_key, "capability pack key").ok()?;
        let pack_id = self.aliases.get(&normalised_key)?;
        self.descriptors.get(pack_id)
    }

    pub fn resolve_stage_owner(&self, stage: &str) -> Option<&str> {
        let normalised_stage = normalise_identifier(stage, "capability stage").ok()?;
        self.stage_owners.get(&normalised_stage).map(String::as_str)
    }

    pub fn resolve_ingester_owner(&self, ingester: &str) -> Option<&str> {
        let normalised_ingester = normalise_identifier(ingester, "capability ingester").ok()?;
        self.ingester_owners
            .get(&normalised_ingester)
            .map(String::as_str)
    }

    pub fn observations(&self) -> &[CapabilityPackRegistrationObservation] {
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
        self.observations
            .push(CapabilityPackRegistrationObservation {
                pack_id: pack_id.to_string(),
                status: CapabilityPackRegistrationStatus::Rejected,
                reason: Some(reason),
            });
    }
}

fn normalise_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, CapabilityPackRegistryError> {
    let normalised = value.trim().to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(CapabilityPackRegistryError::InvalidIdentifier {
            field,
            value: value.to_string(),
        });
    }
    Ok(normalised)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLONES_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
        id: "semantic-clones-pack",
        display_name: "Semantic Clones",
        aliases: &["clones"],
        stage_contributions: &["semantic-clones"],
        ingester_contributions: &["semantic-clones-ingester"],
    };

    const KNOWLEDGE_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
        id: "knowledge-pack",
        display_name: "Knowledge",
        aliases: &["knowledge"],
        stage_contributions: &["knowledge"],
        ingester_contributions: &["knowledge-ingester"],
    };

    #[test]
    fn capability_pack_registry_registers_and_resolves_contributions() {
        let mut registry = CapabilityPackRegistry::new();
        registry
            .register(CLONES_CAPABILITY_PACK)
            .expect("register semantic clones pack");
        registry
            .register(KNOWLEDGE_CAPABILITY_PACK)
            .expect("register knowledge pack");

        assert_eq!(
            registry.resolve_stage_owner("semantic-clones"),
            Some("semantic-clones-pack")
        );
        assert_eq!(
            registry.resolve_ingester_owner("knowledge-ingester"),
            Some("knowledge-pack")
        );
        assert_eq!(
            registry
                .resolve_pack("clones")
                .expect("resolve pack by alias")
                .id,
            "semantic-clones-pack"
        );
    }

    #[test]
    fn capability_pack_registry_rejects_stage_collisions() {
        let mut registry = CapabilityPackRegistry::new();
        registry
            .register(CLONES_CAPABILITY_PACK)
            .expect("register semantic clones pack");

        let error = registry
            .register(CapabilityPackDescriptor {
                id: "another-pack",
                display_name: "Another pack",
                aliases: &[],
                stage_contributions: &["semantic-clones"],
                ingester_contributions: &[],
            })
            .expect_err("duplicate stage ownership should fail");

        assert!(matches!(
            error,
            CapabilityPackRegistryError::StageAlreadyOwned { .. }
        ));
    }

    #[test]
    fn capability_pack_registry_rejects_ingester_collisions() {
        let mut registry = CapabilityPackRegistry::new();
        registry
            .register(KNOWLEDGE_CAPABILITY_PACK)
            .expect("register knowledge pack");

        let error = registry
            .register(CapabilityPackDescriptor {
                id: "another-pack",
                display_name: "Another pack",
                aliases: &[],
                stage_contributions: &["another-stage"],
                ingester_contributions: &["knowledge-ingester"],
            })
            .expect_err("duplicate ingester ownership should fail");

        assert!(matches!(
            error,
            CapabilityPackRegistryError::IngesterAlreadyOwned { .. }
        ));
    }
}
