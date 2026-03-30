use anyhow::{Context, Result, bail};

use crate::host::capability_host::KnowledgeIngestContext;

use super::reference_parse::parse_knowledge_ref;
use super::reference_types::{
    KnowledgeRef, ResolvedKnowledgeSourceRef, ResolvedKnowledgeTargetRef,
};
use super::reference_validate::{is_valid_artefact_id, resolve_commit_sha};

pub fn resolve_source_ref(
    ctx: &dyn KnowledgeIngestContext,
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
    ctx: &dyn KnowledgeIngestContext,
    raw: &str,
) -> Result<ResolvedKnowledgeTargetRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::Commit { rev } => Ok(ResolvedKnowledgeTargetRef::Commit {
            sha: resolve_commit_sha(ctx.repo_root(), &rev)?,
        }),
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id,
            knowledge_item_version_id,
        } => {
            let item = ctx
                .knowledge_relational()
                .find_item_by_id(&ctx.repo().repo_id, &knowledge_item_id)?
                .with_context(|| {
                    format!("target knowledge item `{knowledge_item_id}` not found")
                })?;

            if let Some(target_version_id) = knowledge_item_version_id {
                let version = ctx
                    .knowledge_documents()
                    .find_knowledge_item_version(&target_version_id)?
                    .with_context(|| {
                        format!("target knowledge item version `{target_version_id}` not found")
                    })?;

                if version.knowledge_item_id != knowledge_item_id {
                    bail!(
                        "target knowledge version `{target_version_id}` does not belong to knowledge item `{knowledge_item_id}`"
                    );
                }

                Ok(ResolvedKnowledgeTargetRef::KnowledgeItem {
                    knowledge_item_id,
                    target_knowledge_item_version_id: Some(target_version_id),
                })
            } else {
                let target_knowledge_item_version_id =
                    item.latest_knowledge_item_version_id.trim().to_string();
                if target_knowledge_item_version_id.is_empty() {
                    bail!(
                        "target knowledge item `{knowledge_item_id}` has no latest knowledge item version"
                    );
                }

                Ok(ResolvedKnowledgeTargetRef::KnowledgeItem {
                    knowledge_item_id,
                    target_knowledge_item_version_id: Some(target_knowledge_item_version_id),
                })
            }
        }
        KnowledgeRef::Checkpoint { checkpoint_id } => {
            let resolved = ctx
                .host_relational()
                .resolve_checkpoint_id(&ctx.repo().repo_id, &checkpoint_id)?;
            Ok(ResolvedKnowledgeTargetRef::Checkpoint {
                checkpoint_id: resolved,
            })
        }
        KnowledgeRef::Artefact { artefact_id } => {
            let trimmed = artefact_id.trim();
            if trimmed.is_empty() {
                bail!("artefact id must not be empty");
            }
            if !is_valid_artefact_id(trimmed) {
                bail!(
                    "artefact id `{trimmed}` is not a valid artefact identifier \
                     (expected lowercase UUID)"
                );
            }

            let exists = ctx
                .host_relational()
                .artefact_exists(&ctx.repo().repo_id, trimmed)?;
            if !exists {
                bail!("artefact `{trimmed}` not found");
            }

            Ok(ResolvedKnowledgeTargetRef::Artefact {
                artefact_id: trimmed.to_string(),
            })
        }
        KnowledgeRef::KnowledgeVersion { .. } => {
            bail!("target ref `{raw}` is not supported as a target by `knowledge associate` yet")
        }
    }
}
