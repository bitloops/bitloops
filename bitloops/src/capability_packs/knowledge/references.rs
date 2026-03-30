pub use super::reference_parse::parse_knowledge_ref;
pub use super::reference_resolve::{resolve_source_ref, resolve_target_ref};
pub use super::reference_types::{
    KnowledgeRef, ResolvedKnowledgeSourceRef, ResolvedKnowledgeTargetRef,
};
#[cfg(test)]
pub(crate) use super::reference_validate::is_valid_artefact_id;
pub use super::reference_validate::resolve_commit_sha;

#[cfg(test)]
#[path = "knowledge_references_tests.rs"]
mod tests;
