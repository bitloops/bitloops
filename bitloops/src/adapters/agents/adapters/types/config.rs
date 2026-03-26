use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};

use super::normalise_key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentConfigValueKind {
    String,
    Boolean,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentConfigField {
    pub key: &'static str,
    pub value_kind: AgentConfigValueKind,
    pub required: bool,
}

const EMPTY_CONFIG_FIELDS: &[AgentConfigField] = &[];
const EMPTY_CONFIG_CONFLICTS: &[(&str, &str)] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentConfigSchema {
    pub namespace: &'static str,
    pub fields: &'static [AgentConfigField],
    pub mutually_exclusive: &'static [(&'static str, &'static str)],
}

impl AgentConfigSchema {
    pub const fn empty(namespace: &'static str) -> Self {
        Self {
            namespace,
            fields: EMPTY_CONFIG_FIELDS,
            mutually_exclusive: EMPTY_CONFIG_CONFLICTS,
        }
    }

    pub(crate) fn validate_shape(&self, scope: &str, id: &str) -> Result<()> {
        if self.namespace.trim().is_empty() {
            bail!("{scope} {id} has empty config namespace");
        }

        let mut seen = HashSet::new();
        for field in self.fields {
            let key = normalise_key(field.key)?;
            if !seen.insert(key.clone()) {
                bail!("{scope} {id} has duplicate config field key: {key}");
            }
        }

        for (left, right) in self.mutually_exclusive {
            let left_key = normalise_key(left)?;
            let right_key = normalise_key(right)?;
            if left_key == right_key {
                bail!(
                    "{scope} {id} has invalid mutually exclusive config pair: {}",
                    left.trim()
                );
            }
        }

        Ok(())
    }

    pub(crate) fn validate_values(
        &self,
        scope: &str,
        id: &str,
        values: Option<&HashMap<String, String>>,
    ) -> Vec<AgentConfigValidationIssue> {
        let mut issues = Vec::new();
        let empty = HashMap::new();
        let values = values.unwrap_or(&empty);

        for field in self.fields {
            let key = field.key.trim().to_ascii_lowercase();
            let value = values.get(&key).map(|v| v.trim().to_string());

            if field.required && value.as_deref().unwrap_or_default().is_empty() {
                issues.push(AgentConfigValidationIssue {
                    scope: scope.to_string(),
                    id: id.to_string(),
                    namespace: self.namespace.to_string(),
                    field: field.key.to_string(),
                    code: "missing_required_config".to_string(),
                    message: format!(
                        "missing required config field '{}.{}'",
                        self.namespace, field.key
                    ),
                });
                continue;
            }

            let Some(value) = value else {
                continue;
            };
            if value.is_empty() {
                continue;
            }

            let valid = match field.value_kind {
                AgentConfigValueKind::String | AgentConfigValueKind::Path => !value.is_empty(),
                AgentConfigValueKind::Boolean => matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "false" | "1" | "0" | "yes" | "no"
                ),
            };

            if !valid {
                issues.push(AgentConfigValidationIssue {
                    scope: scope.to_string(),
                    id: id.to_string(),
                    namespace: self.namespace.to_string(),
                    field: field.key.to_string(),
                    code: "invalid_config_value".to_string(),
                    message: format!(
                        "invalid value for config field '{}.{}'",
                        self.namespace, field.key
                    ),
                });
            }
        }

        for (left, right) in self.mutually_exclusive {
            let left_key = left.trim().to_ascii_lowercase();
            let right_key = right.trim().to_ascii_lowercase();
            let left_set = values
                .get(&left_key)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            let right_set = values
                .get(&right_key)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            if left_set && right_set {
                issues.push(AgentConfigValidationIssue {
                    scope: scope.to_string(),
                    id: id.to_string(),
                    namespace: self.namespace.to_string(),
                    field: format!("{left},{right}"),
                    code: "conflicting_config".to_string(),
                    message: format!(
                        "config fields '{}.{}' and '{}.{}' cannot both be set",
                        self.namespace, left, self.namespace, right
                    ),
                });
            }
        }

        issues
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentAdapterConfiguration {
    pub families: HashMap<String, HashMap<String, String>>,
    pub profiles: HashMap<String, HashMap<String, String>>,
    pub adapters: HashMap<String, HashMap<String, String>>,
}

impl AgentAdapterConfiguration {
    pub fn with_family_value(
        mut self,
        family_id: impl AsRef<str>,
        key: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Self {
        let family_id = family_id.as_ref().trim().to_ascii_lowercase();
        let key = key.as_ref().trim().to_ascii_lowercase();
        self.families
            .entry(family_id)
            .or_default()
            .insert(key, value.as_ref().trim().to_string());
        self
    }

    pub fn with_profile_value(
        mut self,
        profile_id: impl AsRef<str>,
        key: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Self {
        let profile_id = profile_id.as_ref().trim().to_ascii_lowercase();
        let key = key.as_ref().trim().to_ascii_lowercase();
        self.profiles
            .entry(profile_id)
            .or_default()
            .insert(key, value.as_ref().trim().to_string());
        self
    }

    pub fn with_adapter_value(
        mut self,
        adapter_id: impl AsRef<str>,
        key: impl AsRef<str>,
        value: impl AsRef<str>,
    ) -> Self {
        let adapter_id = adapter_id.as_ref().trim().to_ascii_lowercase();
        let key = key.as_ref().trim().to_ascii_lowercase();
        self.adapters
            .entry(adapter_id)
            .or_default()
            .insert(key, value.as_ref().trim().to_string());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfigValidationIssue {
    pub scope: String,
    pub id: String,
    pub namespace: String,
    pub field: String,
    pub code: String,
    pub message: String,
}
