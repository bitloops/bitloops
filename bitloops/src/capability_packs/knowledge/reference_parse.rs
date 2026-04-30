use anyhow::{Context, Result, bail};

use super::reference_types::KnowledgeRef;

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
        "path" => Ok(KnowledgeRef::Path {
            path: value.to_string(),
        }),
        "symbol_fqn" => Ok(KnowledgeRef::SymbolFqn {
            symbol_fqn: value.to_string(),
        }),
        _ => bail!("unsupported knowledge ref kind `{kind}`"),
    }
}
