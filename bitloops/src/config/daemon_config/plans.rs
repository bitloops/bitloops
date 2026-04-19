use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::{DocumentMut, Item};

use super::toml::{ensure_child_table, ensure_table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonEmbeddingsInstallMode {
    Bootstrap,
    WarmExisting,
    SkipHosted,
}

#[derive(Debug, Clone)]
pub(crate) struct DaemonEmbeddingsInstallPlan {
    pub config_path: PathBuf,
    pub profile_name: String,
    pub runtime_name: String,
    pub profile_driver: Option<String>,
    pub mode: DaemonEmbeddingsInstallMode,
    pub config_modified: bool,
    pub(crate) original_contents: Option<String>,
    pub(crate) prepared_contents: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DaemonInferenceInstallPlan {
    pub config_path: PathBuf,
    pub config_modified: bool,
    pub(crate) original_contents: Option<String>,
    pub(crate) prepared_contents: Option<String>,
}

impl DaemonEmbeddingsInstallPlan {
    pub fn apply(&self) -> Result<()> {
        self.write_prepared_contents(self.prepared_contents.as_deref())
    }

    pub fn apply_with_managed_runtime_path(&self, binary_path: &Path) -> Result<()> {
        let staged_contents = self
            .prepared_contents
            .as_deref()
            .or(self.original_contents.as_deref())
            .unwrap_or_default();
        let mut staged_doc = if staged_contents.trim().is_empty() {
            DocumentMut::new()
        } else {
            staged_contents.parse::<DocumentMut>().with_context(|| {
                format!(
                    "parsing staged Bitloops daemon config {}",
                    self.config_path.display()
                )
            })?
        };
        let inference = ensure_table(&mut staged_doc, "inference");
        let runtimes = ensure_child_table(inference, "runtimes");
        let runtime = ensure_child_table(runtimes, &self.runtime_name);
        runtime["command"] = Item::Value(binary_path.to_string_lossy().to_string().into());

        let desired_runtime = staged_doc
            .get("inference")
            .and_then(Item::as_table)
            .and_then(|table| table.get("runtimes"))
            .and_then(Item::as_table)
            .and_then(|table| table.get(&self.runtime_name))
            .cloned()
            .context("staged embeddings runtime missing from prepared config")?;
        let desired_profile = staged_doc
            .get("inference")
            .and_then(Item::as_table)
            .and_then(|table| table.get("profiles"))
            .and_then(Item::as_table)
            .and_then(|table| table.get(&self.profile_name))
            .cloned()
            .with_context(|| {
                format!(
                    "staged embeddings profile `{}` missing from prepared config",
                    self.profile_name
                )
            })?;
        let desired_code_embeddings = staged_doc
            .get("semantic_clones")
            .and_then(Item::as_table)
            .and_then(|table| table.get("inference"))
            .and_then(Item::as_table)
            .and_then(|table| table.get("code_embeddings"))
            .cloned()
            .context("staged code_embeddings binding missing from prepared config")?;
        let desired_summary_embeddings = staged_doc
            .get("semantic_clones")
            .and_then(Item::as_table)
            .and_then(|table| table.get("inference"))
            .and_then(Item::as_table)
            .and_then(|table| table.get("summary_embeddings"))
            .cloned()
            .context("staged summary_embeddings binding missing from prepared config")?;

        let current_contents = match fs::read_to_string(&self.config_path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "reading current Bitloops daemon config {}",
                        self.config_path.display()
                    )
                });
            }
        };
        let mut current_doc = if current_contents.trim().is_empty() {
            DocumentMut::new()
        } else {
            current_contents.parse::<DocumentMut>().with_context(|| {
                format!(
                    "parsing current Bitloops daemon config {}",
                    self.config_path.display()
                )
            })?
        };

        {
            let inference = ensure_table(&mut current_doc, "inference");
            let runtimes = ensure_child_table(inference, "runtimes");
            runtimes[&self.runtime_name] = desired_runtime;
            let profiles = ensure_child_table(inference, "profiles");
            profiles[&self.profile_name] = desired_profile;
        }

        {
            let semantic_clones = ensure_table(&mut current_doc, "semantic_clones");
            let inference = ensure_child_table(semantic_clones, "inference");
            inference["code_embeddings"] = desired_code_embeddings;
            inference["summary_embeddings"] = desired_summary_embeddings;
        }

        let updated_contents = current_doc.to_string();
        if current_contents == updated_contents {
            return Ok(());
        }

        fs::write(&self.config_path, updated_contents).with_context(|| {
            format!(
                "writing Bitloops daemon config {}",
                self.config_path.display()
            )
        })
    }

    pub fn rollback(&self) -> Result<()> {
        if !self.config_modified {
            return Ok(());
        }

        match &self.original_contents {
            Some(contents) => fs::write(&self.config_path, contents).with_context(|| {
                format!(
                    "restoring Bitloops daemon config after failed embeddings install {}",
                    self.config_path.display()
                )
            })?,
            None => {
                if self.config_path.exists() {
                    fs::remove_file(&self.config_path).with_context(|| {
                        format!(
                            "removing Bitloops daemon config after failed embeddings install {}",
                            self.config_path.display()
                        )
                    })?;
                }
            }
        }

        Ok(())
    }

    fn write_prepared_contents(&self, contents: Option<&str>) -> Result<()> {
        if !self.config_modified {
            return Ok(());
        }

        let Some(contents) = contents else {
            return Ok(());
        };

        fs::write(&self.config_path, contents).with_context(|| {
            format!(
                "writing Bitloops daemon config {}",
                self.config_path.display()
            )
        })
    }
}

impl DaemonInferenceInstallPlan {
    pub fn apply(&self) -> Result<()> {
        self.write_prepared_contents(self.prepared_contents.as_deref())
    }

    pub fn rollback(&self) -> Result<()> {
        if !self.config_modified {
            return Ok(());
        }

        match &self.original_contents {
            Some(contents) => fs::write(&self.config_path, contents).with_context(|| {
                format!(
                    "restoring Bitloops daemon config after failed inference install {}",
                    self.config_path.display()
                )
            })?,
            None => {
                if self.config_path.exists() {
                    fs::remove_file(&self.config_path).with_context(|| {
                        format!(
                            "removing Bitloops daemon config after failed inference install {}",
                            self.config_path.display()
                        )
                    })?;
                }
            }
        }

        Ok(())
    }

    fn write_prepared_contents(&self, contents: Option<&str>) -> Result<()> {
        if !self.config_modified {
            return Ok(());
        }

        let Some(contents) = contents else {
            return Ok(());
        };

        fs::write(&self.config_path, contents).with_context(|| {
            format!(
                "writing Bitloops daemon config {}",
                self.config_path.display()
            )
        })
    }
}
