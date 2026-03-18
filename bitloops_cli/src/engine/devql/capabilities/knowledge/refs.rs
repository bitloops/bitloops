use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::OptionalExtension;

use crate::engine::db::SqliteConnectionPool;
use crate::engine::strategy::manual_commit::run_git;
use crate::engine::trailers::is_valid_checkpoint_id;

use super::types::KnowledgeHostContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeRef {
    KnowledgeItem {
        knowledge_item_id: String,
        knowledge_item_version_id: Option<String>,
    },
    KnowledgeVersion {
        document_version_id: String,
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
    pub source_document_version_id: String,
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
            let document_version_id = version.trim();
            if knowledge_item_id.is_empty() || document_version_id.is_empty() {
                bail!("knowledge ref must use `knowledge:<item_id>[:<version_id>]`");
            }
            Ok((
                knowledge_item_id.to_string(),
                Some(document_version_id.to_string()),
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
            document_version_id: value.to_string(),
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
    host: &KnowledgeHostContext,
    raw: &str,
) -> Result<ResolvedKnowledgeSourceRef> {
    match parse_knowledge_ref(raw)? {
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id,
            knowledge_item_version_id,
        } => {
            let item = host
                .relational_store
                .find_item_by_id(&host.repo.repo_id, &knowledge_item_id)?
                .with_context(|| format!("knowledge item `{knowledge_item_id}` not found"))?;

            if let Some(source_document_version_id) = knowledge_item_version_id {
                let version = host
                    .document_store
                    .find_document_version(&source_document_version_id)?
                    .with_context(|| {
                        format!(
                            "knowledge document version `{source_document_version_id}` not found"
                        )
                    })?;

                if version.knowledge_item_id != knowledge_item_id {
                    bail!(
                        "knowledge version `{source_document_version_id}` does not belong to knowledge item `{knowledge_item_id}`"
                    );
                }

                Ok(ResolvedKnowledgeSourceRef {
                    knowledge_item_id,
                    source_document_version_id,
                })
            } else {
                let source_document_version_id = item.latest_document_version_id.trim().to_string();
                if source_document_version_id.is_empty() {
                    bail!("knowledge item `{knowledge_item_id}` has no latest document version");
                }

                Ok(ResolvedKnowledgeSourceRef {
                    knowledge_item_id,
                    source_document_version_id,
                })
            }
        }
        KnowledgeRef::KnowledgeVersion {
            document_version_id,
        } => {
            eprintln!(
                "warning: `knowledge_version:<id>` is deprecated; use `knowledge:<knowledge_item_id>:<document_version_id>`"
            );
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
        KnowledgeRef::Checkpoint { .. } => {
            bail!("`checkpoint:<id>` cannot be used as a knowledge association source")
        }
        KnowledgeRef::Artefact { .. } => {
            bail!("`artefact:<id>` cannot be used as a knowledge association source")
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
        KnowledgeRef::KnowledgeItem {
            knowledge_item_id,
            knowledge_item_version_id: None,
        } => {
            host.relational_store
                .find_item_by_id(&host.repo.repo_id, &knowledge_item_id)?
                .with_context(|| {
                    format!("target knowledge item `{knowledge_item_id}` not found")
                })?;
            Ok(ResolvedKnowledgeTargetRef::KnowledgeItem { knowledge_item_id })
        }
        KnowledgeRef::Checkpoint { checkpoint_id } => {
            let sqlite_path = host
                .backends
                .relational
                .resolve_sqlite_db_path()
                .context("resolving SQLite path for checkpoint resolution")?;
            let validated =
                resolve_checkpoint_id(&sqlite_path, &host.repo.repo_id, &checkpoint_id)?;
            Ok(ResolvedKnowledgeTargetRef::Checkpoint {
                checkpoint_id: validated,
            })
        }
        KnowledgeRef::Artefact { artefact_id } => {
            let sqlite_path = host
                .backends
                .relational
                .resolve_sqlite_db_path()
                .context("resolving SQLite path for artefact resolution")?;
            let validated = resolve_artefact_id(&sqlite_path, &host.repo.repo_id, &artefact_id)?;
            Ok(ResolvedKnowledgeTargetRef::Artefact {
                artefact_id: validated,
            })
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

pub fn resolve_checkpoint_id(
    sqlite_path: &Path,
    repo_id: &str,
    checkpoint_id: &str,
) -> Result<String> {
    let trimmed = checkpoint_id.trim();
    if trimmed.is_empty() {
        bail!("checkpoint id must not be empty");
    }
    if !is_valid_checkpoint_id(trimmed) {
        bail!(
            "checkpoint id `{trimmed}` is not a valid checkpoint identifier \
             (expected 12-character lowercase hex)"
        );
    }

    let pool = SqliteConnectionPool::connect(sqlite_path.to_path_buf())
        .context("opening checkpoint database for checkpoint resolution")?;
    pool.initialise_checkpoint_schema()
        .context("initialising checkpoint schema for checkpoint resolution")?;

    let exists = pool.with_connection(|conn| {
        conn.query_row(
            "SELECT checkpoint_id FROM checkpoints WHERE checkpoint_id = ?1 AND repo_id = ?2 LIMIT 1",
            rusqlite::params![trimmed, repo_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(anyhow::Error::from)
    })?;

    exists
        .map(|id| id.trim().to_string())
        .with_context(|| format!("checkpoint `{trimmed}` not found in current repository"))
}

pub fn resolve_artefact_id(sqlite_path: &Path, repo_id: &str, artefact_id: &str) -> Result<String> {
    let trimmed = artefact_id.trim();
    if trimmed.is_empty() {
        bail!("artefact id must not be empty");
    }
    if !is_valid_artefact_id(trimmed) {
        bail!(
            "artefact id `{trimmed}` is not a valid artefact identifier \
             (expected deterministic UUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)"
        );
    }

    let pool = SqliteConnectionPool::connect(sqlite_path.to_path_buf())
        .context("opening database for artefact resolution")?;

    let exists = pool.with_connection(|conn| {
        let current = conn
            .query_row(
                "SELECT artefact_id FROM artefacts_current \
                 WHERE repo_id = ?1 AND artefact_id = ?2 LIMIT 1",
                rusqlite::params![repo_id, trimmed],
                |row| row.get::<_, String>(0),
            )
            .optional();

        match current {
            Ok(Some(id)) => return Ok(Some(id)),
            Ok(None) => {}
            Err(e) if e.to_string().contains("no such table") => {}
            Err(e) => return Err(anyhow::Error::from(e)),
        }

        let historical = conn
            .query_row(
                "SELECT artefact_id FROM artefacts \
                 WHERE repo_id = ?1 AND artefact_id = ?2 LIMIT 1",
                rusqlite::params![repo_id, trimmed],
                |row| row.get::<_, String>(0),
            )
            .optional();

        match historical {
            Ok(result) => Ok(result),
            Err(e) if e.to_string().contains("no such table") => Ok(None),
            Err(e) => Err(anyhow::Error::from(e)),
        }
    })?;

    exists
        .map(|id| id.trim().to_string())
        .with_context(|| format!("artefact `{trimmed}` not found in current repository"))
}

fn is_valid_artefact_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected_lengths = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected_lengths.iter())
        .all(|(part, &len)| {
            part.len() == len
                && part
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
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
                knowledge_item_id: "item-1".to_string(),
                knowledge_item_version_id: None
            }
        );
    }

    #[test]
    fn parses_knowledge_item_ref_with_version() {
        let parsed =
            parse_knowledge_ref("knowledge:item-1:version-1").expect("versioned knowledge ref");
        assert_eq!(
            parsed,
            KnowledgeRef::KnowledgeItem {
                knowledge_item_id: "item-1".to_string(),
                knowledge_item_version_id: Some("version-1".to_string())
            }
        );
    }

    #[test]
    fn parses_knowledge_version_ref_legacy_alias() {
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
    fn parses_checkpoint_ref() {
        let parsed = parse_knowledge_ref("checkpoint:a1b2c3d4e5f6").expect("checkpoint ref");
        assert_eq!(
            parsed,
            KnowledgeRef::Checkpoint {
                checkpoint_id: "a1b2c3d4e5f6".to_string()
            }
        );
    }

    #[test]
    fn rejects_unknown_knowledge_ref_kind() {
        let err = parse_knowledge_ref("webhook:abc123").expect_err("unknown kind must fail");
        assert!(err.to_string().contains("unsupported knowledge ref kind"));
    }

    #[test]
    fn rejects_missing_knowledge_ref_value() {
        let err = parse_knowledge_ref("knowledge:").expect_err("missing value must fail");
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn rejects_knowledge_ref_missing_item_segment() {
        let err = parse_knowledge_ref("knowledge::version-1")
            .expect_err("missing item segment must fail");
        assert!(err.to_string().contains("knowledge:<item_id>"));
    }

    #[test]
    fn rejects_knowledge_ref_missing_version_segment() {
        let err = parse_knowledge_ref("knowledge:item-1:")
            .expect_err("missing version segment must fail");
        assert!(err.to_string().contains("knowledge:<item_id>"));
    }

    #[test]
    fn rejects_knowledge_ref_with_too_many_segments() {
        let err = parse_knowledge_ref("knowledge:a:b:c").expect_err("too many segments must fail");
        assert!(err.to_string().contains("knowledge:<item_id>"));
    }

    #[test]
    fn rejects_missing_knowledge_ref_separator() {
        let err = parse_knowledge_ref("knowledge").expect_err("missing separator must fail");
        assert!(err.to_string().contains("`<kind>:<value>`"));
    }

    #[test]
    fn resolve_target_ref_rejects_knowledge_version_as_target() {
        let parsed = parse_knowledge_ref("knowledge_version:some-version-id");
        assert!(
            parsed.is_ok(),
            "knowledge_version should parse successfully"
        );
        assert_eq!(
            parsed.unwrap(),
            KnowledgeRef::KnowledgeVersion {
                document_version_id: "some-version-id".to_string()
            }
        );
    }

    #[test]
    fn parses_artefact_ref() {
        let parsed = parse_knowledge_ref("artefact:a1b2c3d4-e5f6-7890-abcd-ef1234567890")
            .expect("artefact ref");
        assert_eq!(
            parsed,
            KnowledgeRef::Artefact {
                artefact_id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string()
            }
        );
    }

    #[test]
    fn is_valid_artefact_id_accepts_well_formed_uuid() {
        assert!(is_valid_artefact_id("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
    }

    #[test]
    fn is_valid_artefact_id_rejects_uppercase() {
        assert!(!is_valid_artefact_id(
            "A1B2C3D4-E5F6-7890-ABCD-EF1234567890"
        ));
    }

    #[test]
    fn is_valid_artefact_id_rejects_wrong_length() {
        assert!(!is_valid_artefact_id("a1b2c3d4-e5f6-7890-abcd-ef12345678"));
    }

    #[test]
    fn is_valid_artefact_id_rejects_missing_dashes() {
        assert!(!is_valid_artefact_id("a1b2c3d4e5f67890abcdef1234567890"));
    }
}
