use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuntimeFileConfig {
    #[serde(default)]
    pub embeddings: EmbeddingsSectionConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EmbeddingsSectionConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, EmbeddingProfileConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmbeddingProfileConfig {
    #[serde(rename = "local_fastembed")]
    LocalFastembed {
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        cache_dir: Option<PathBuf>,
    },
    #[serde(rename = "openai")]
    OpenAi {
        model: String,
        api_key: String,
        #[serde(default)]
        base_url: Option<String>,
    },
    #[serde(rename = "voyage")]
    Voyage {
        model: String,
        api_key: String,
        #[serde(default)]
        base_url: Option<String>,
    },
}

impl EmbeddingProfileConfig {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::LocalFastembed { .. } => "local_fastembed",
            Self::OpenAi { .. } => "openai",
            Self::Voyage { .. } => "voyage",
        }
    }
}

pub fn load_runtime_file_config(path: &Path) -> Result<RuntimeFileConfig> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("reading embeddings runtime config {}", path.display()))?;
    toml::from_str(&data)
        .with_context(|| format!("parsing embeddings runtime config {}", path.display()))
}

pub fn select_profile<'a>(
    config: &'a RuntimeFileConfig,
    selected_profile: &str,
) -> Result<&'a EmbeddingProfileConfig> {
    let key = selected_profile.trim();
    if key.is_empty() {
        bail!("selected embedding profile cannot be empty");
    }

    config
        .embeddings
        .profiles
        .get(key)
        .ok_or_else(|| anyhow!("embedding profile `{key}` was not found in runtime config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_file_config_parses_profile_sections() {
        let config: RuntimeFileConfig = toml::from_str(
            r#"
[embeddings.profiles.local]
kind = "local_fastembed"
model = "jinaai/jina-embeddings-v2-base-code"

[embeddings.profiles.openai]
kind = "openai"
model = "text-embedding-3-large"
api_key = "secret"
"#,
        )
        .expect("parse runtime config");

        assert_eq!(config.embeddings.profiles.len(), 2);
        assert!(matches!(
            config.embeddings.profiles.get("local"),
            Some(EmbeddingProfileConfig::LocalFastembed { .. })
        ));
    }
}
