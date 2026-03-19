use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::engine::devql::capability_host::CapabilityIngestContext;
use crate::engine::strategy::manual_commit::run_git;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeRef {
    KnowledgeItem {
        knowledge_item_id: String,
        knowledge_item_version_id: Option<String>,
    },
    KnowledgeVersion {
        knowledge_item_version_id: String,
    },
    Commit {
        rev: String,
    },
    Checkpoint {
        checkpoint_id: String,
    },
    Artefact {
        artefact_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKnowledgeSourceRef {
    pub knowledge_item_id: String,
    pub source_knowledge_item_version_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedKnowledgeTargetRef {
    Commit { sha: String },
    KnowledgeItem { knowledge_item_id: String },
    Checkpoint { checkpoint_id: String },
    Artefact { artefact_id: String },
}

fn parse_knowledge_source_value(value: &str) -> Result<(String, Option<String>)> {
    let segments: Vec<&str> = value.split(':').collect();
    match segments.as_slice() {
        [item] => {
            let knowledge_item_id = item.trim();
            if knowledge_item_id.is_empty() {
                bail!("knowledge ref value must not be empty");
            }
            Ok((knowledge_item_id.to_string(), None))
        }
        [item, version] => {
            let knowledge_item_id = item.trim();
            let knowledge_item_version_id = version.trim();
            if knowledge_item_id.is_empty() || knowledge_item_version_id.is_empty() {
                bail!("knowledge ref must use `knowledge:<item_id>[:<version_id>]`");
            }
            Ok((
                knowledge_item_id.to_string(),
                Some(knowledge_item_version_id.to_string()),
            ))
        }
        _ => bail!(
            "knowledge ref must use `knowledge:<item_id>` or `knowledge:<item_id>:<version_id>`"
        ),
    }
}

pub fn parse_knowledge_ref(raw: &str) -> Result<KnowledgeRef> {
    let trimmed = raw.trim();
    let (kind, value) = trimmed
        .split_once(':')
        .context("knowledge ref must use `<kind>:<value>` syntax")?;
    let value = value.trim();
    if value.is_empty() {
        bail!("knowledge ref value must not be empty");
    }

    match kind {
        "knowledge" => {
            let (knowledge_item_id, knowledge_item_version_id) =
                parse_knowledge_source_value(value)?;
            Ok(KnowledgeRef::KnowledgeItem {
                knowledge_item_id,
                knowledge_item_version_id,
            })
        }
        "knowledge_version" => Ok(KnowledgeRef::KnowledgeVersion {
            knowledge_item_version_id: value.to_string(),
        }),
        "commit" => Ok(KnowledgeRef::Commit {
            rev: value.to_string(),
        }),
        "checkpoint" => Ok(KnowledgeRef::Checkpoint {
            checkpoint_id: value.to_string(),
        }),
        "artefact" => Ok(KnowledgeRef::Artefact {
            artefact_id: value.to_string(),
        }),
        _ => bail!("unsupported knowledge ref kind `{kind}`"),
    }
}

pub fn resolve_source_ref(
    ctx: &dyn CapabilityIngestContext,
    raw: &str,
) -> Result<ResolvedKnowledgeSourceRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id,
            knowledge_item_version_id,
        } => {
            let item = ctx
                .knowledge_relational()
                .find_item_by_id(&ctx.repo().repo_id, &knowledge_item_id)?
                .with_context(|| format!("knowledge item `{knowledge_item_id}` not found"))?;

            if let Some(source_knowledge_item_version_id) = knowledge_item_version_id {
                let version = ctx
                    .knowledge_documents()
                    .find_knowledge_item_version(&source_knowledge_item_version_id)?
                    .with_context(|| {
                        format!(
                            "knowledge item version `{source_knowledge_item_version_id}` not found"
                        )
                    })?;

                if version.knowledge_item_id != knowledge_item_id {
                    bail!(
                        "knowledge version `{source_knowledge_item_version_id}` does not belong to knowledge item `{knowledge_item_id}`"
                    );
                }

                Ok(ResolvedKnowledgeSourceRef {
                    knowledge_item_id,
                    source_knowledge_item_version_id,
                })
            } else {
                let source_knowledge_item_version_id =
                    item.latest_knowledge_item_version_id.trim().to_string();
                if source_knowledge_item_version_id.is_empty() {
                    bail!(
                        "knowledge item `{knowledge_item_id}` has no latest knowledge item version"
                    );
                }

                Ok(ResolvedKnowledgeSourceRef {
                    knowledge_item_id,
                    source_knowledge_item_version_id,
                })
            }
        }
        KnowledgeRef::KnowledgeVersion {
            knowledge_item_version_id,
        } => {
            eprintln!(
                "warning: `knowledge_version:<id>` is deprecated; use `knowledge:<knowledge_item_id>:<knowledge_item_version_id>`"
            );
            let version = ctx
                .knowledge_documents()
                .find_knowledge_item_version(&knowledge_item_version_id)?
                .with_context(|| {
                    format!("knowledge item version `{knowledge_item_version_id}` not found")
                })?;
            ctx.knowledge_relational()
                .find_item_by_id(&ctx.repo().repo_id, &version.knowledge_item_id)?
                .with_context(|| {
                    format!(
                        "knowledge item `{}` for knowledge item version `{knowledge_item_version_id}` not found in current repo",
                        version.knowledge_item_id
                    )
                })?;

            Ok(ResolvedKnowledgeSourceRef {
                knowledge_item_id: version.knowledge_item_id,
                source_knowledge_item_version_id: knowledge_item_version_id,
            })
        }
        KnowledgeRef::Commit { .. } => {
            bail!("`commit:<sha>` cannot be used as a knowledge association source")
        }
        KnowledgeRef::Checkpoint { .. } => {
            bail!("`checkpoint:<id>` cannot be used as a knowledge association source")
        }
        KnowledgeRef::Artefact { .. } => {
            bail!("`artefact:<id>` cannot be used as a knowledge association source")
        }
    }
}

pub fn resolve_target_ref(
    ctx: &dyn CapabilityIngestContext,
    raw: &str,
) -> Result<ResolvedKnowledgeTargetRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::Commit { rev } => Ok(ResolvedKnowledgeTargetRef::Commit {
            sha: resolve_commit_sha(ctx.repo_root(), &rev)?,
        }),
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id,
            knowledge_item_version_id: None,
        } => {
            ctx.knowledge_relational()
                .find_item_by_id(&ctx.repo().repo_id, &knowledge_item_id)?
                .with_context(|| format!("target knowledge item `{knowledge_item_id}` not found"))?;
            Ok(ResolvedKnowledgeTargetRef::KnowledgeItem { knowledge_item_id })
        }
        KnowledgeRef::Checkpoint { checkpoint_id } => {
            let resolved = ctx
                .knowledge_relational()
                .resolve_checkpoint_id(&ctx.repo().repo_id, &checkpoint_id)?;
            Ok(ResolvedKnowledgeTargetRef::Checkpoint {
                checkpoint_id: resolved,
            })
        }
        KnowledgeRef::Artefact { artefact_id } => {
            let exists = ctx
                .knowledge_relational()
                .artefact_exists(&ctx.repo().repo_id, &artefact_id)?;
            if !exists {
                bail!("artefact `{artefact_id}` not found");
            }
            Ok(ResolvedKnowledgeTargetRef::Artefact { artefact_id })
        }
        KnowledgeRef::KnowledgeItem {
            knowledge_item_version_id: Some(_),
            ..
        }
        | KnowledgeRef::KnowledgeVersion { .. } => {
            bail!("target ref `{raw}` is not supported as a target by `knowledge associate` yet")
        }
    }
}

pub fn resolve_commit_sha(repo_root: &Path, rev: &str) -> Result<String> {
    let trimmed = rev.trim();
    if trimmed.is_empty() {
        bail!("commit sha must not be empty");
    }

    let resolved = run_git(
        repo_root,
        &["rev-parse", "--verify", &format!("{trimmed}^{{commit}}")],
    )
    .with_context(|| format!("validating commit `{trimmed}`"))?;
    Ok(resolved.trim().to_string())
}
