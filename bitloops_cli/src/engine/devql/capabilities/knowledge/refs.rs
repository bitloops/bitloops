use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::engine::strategy::manual_commit::run_git;

use super::types::KnowledgeHostContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeRef {
    KnowledgeItem { knowledge_item_id: String },
    KnowledgeVersion { document_version_id: String },
    Commit { rev: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKnowledgeSourceRef {
    pub knowledge_item_id: String,
    pub source_document_version_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedKnowledgeTargetRef {
    Commit { sha: String },
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
        "knowledge" => Ok(KnowledgeRef::KnowledgeItem {
            knowledge_item_id: value.to_string(),
        }),
        "knowledge_version" => Ok(KnowledgeRef::KnowledgeVersion {
            document_version_id: value.to_string(),
        }),
        "commit" => Ok(KnowledgeRef::Commit {
            rev: value.to_string(),
        }),
        _ => bail!("unsupported knowledge ref kind `{kind}`"),
    }
}

pub fn resolve_source_ref(
    host: &KnowledgeHostContext,
    raw: &str,
) -> Result<ResolvedKnowledgeSourceRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::KnowledgeItem { knowledge_item_id } => {
            let item = host
                .relational_store
                .find_item_by_id(&host.repo.repo_id, &knowledge_item_id)?
                .with_context(|| format!("knowledge item `{knowledge_item_id}` not found"))?;
            let source_document_version_id = item.latest_document_version_id.trim().to_string();
            if source_document_version_id.is_empty() {
                bail!("knowledge item `{knowledge_item_id}` has no latest document version");
            }

            Ok(ResolvedKnowledgeSourceRef {
                knowledge_item_id,
                source_document_version_id,
            })
        }
        KnowledgeRef::KnowledgeVersion { document_version_id } => {
            let version = host
                .document_store
                .find_document_version(&document_version_id)?
                .with_context(|| {
                    format!("knowledge document version `{document_version_id}` not found")
                })?;
            host.relational_store
                .find_item_by_id(&host.repo.repo_id, &version.knowledge_item_id)?
                .with_context(|| {
                    format!(
                        "knowledge item `{}` for document version `{document_version_id}` not found in current repo",
                        version.knowledge_item_id
                    )
                })?;

            Ok(ResolvedKnowledgeSourceRef {
                knowledge_item_id: version.knowledge_item_id,
                source_document_version_id: document_version_id,
            })
        }
        KnowledgeRef::Commit { .. } => {
            bail!("`commit:<sha>` cannot be used as a knowledge association source")
        }
    }
}

pub fn resolve_target_ref(
    host: &KnowledgeHostContext,
    raw: &str,
) -> Result<ResolvedKnowledgeTargetRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::Commit { rev } => Ok(ResolvedKnowledgeTargetRef::Commit {
            sha: resolve_commit_sha(&host.repo_root, &rev)?,
        }),
        KnowledgeRef::KnowledgeItem { .. } | KnowledgeRef::KnowledgeVersion { .. } => {
            bail!("target ref `{raw}` is not supported by `knowledge associate` yet")
        }
    }
}

pub fn resolve_commit_sha(repo_root: &Path, rev: &str) -> Result<String> {
    let trimmed = rev.trim();
    if trimmed.is_empty() {
        bail!("commit sha must not be empty");
    }

    let resolved = run_git(repo_root, &["rev-parse", "--verify", &format!("{trimmed}^{{commit}}")])
        .with_context(|| format!("validating commit `{trimmed}`"))?;
    Ok(resolved.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_knowledge_item_ref() {
        let parsed = parse_knowledge_ref("knowledge:item-1").expect("knowledge ref");
        assert_eq!(
            parsed,
            KnowledgeRef::KnowledgeItem {
                knowledge_item_id: "item-1".to_string()
            }
        );
    }

    #[test]
    fn parses_knowledge_version_ref() {
        let parsed =
            parse_knowledge_ref("knowledge_version:version-1").expect("knowledge version ref");
        assert_eq!(
            parsed,
            KnowledgeRef::KnowledgeVersion {
                document_version_id: "version-1".to_string()
            }
        );
    }

    #[test]
    fn parses_commit_ref() {
        let parsed = parse_knowledge_ref("commit:abc123").expect("commit ref");
        assert_eq!(
            parsed,
            KnowledgeRef::Commit {
                rev: "abc123".to_string()
            }
        );
    }

    #[test]
    fn rejects_unknown_knowledge_ref_kind() {
        let err = parse_knowledge_ref("checkpoint:abc123").expect_err("unknown kind must fail");
        assert!(err.to_string().contains("unsupported knowledge ref kind"));
    }

    #[test]
    fn rejects_missing_knowledge_ref_value() {
        let err = parse_knowledge_ref("knowledge:").expect_err("missing value must fail");
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn rejects_missing_knowledge_ref_separator() {
        let err = parse_knowledge_ref("knowledge").expect_err("missing separator must fail");
        assert!(err.to_string().contains("`<kind>:<value>`"));
    }
}
