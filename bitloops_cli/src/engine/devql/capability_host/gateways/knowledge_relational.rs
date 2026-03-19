use anyhow::Result;

use crate::engine::devql::capabilities::knowledge::storage::{
    KnowledgeItemRow, KnowledgeRelationAssertionRow, KnowledgeSourceRow,
};

pub trait KnowledgeRelationalGateway: Send + Sync {
    fn initialise_schema(&self) -> Result<()>;
    fn persist_ingestion(&self, source: &KnowledgeSourceRow, item: &KnowledgeItemRow) -> Result<()>;
    fn insert_relation_assertion(
        &self,
        relation: &KnowledgeRelationAssertionRow,
    ) -> Result<()>;
    fn find_item(&self, repo_id: &str, source_id: &str) -> Result<Option<KnowledgeItemRow>>;
    fn find_item_by_id(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItemRow>>;
    fn find_source_by_id(&self, knowledge_source_id: &str) -> Result<Option<KnowledgeSourceRow>>;
    fn list_items_for_repo(&self, repo_id: &str, limit: usize) -> Result<Vec<KnowledgeItemRow>>;
    fn resolve_checkpoint_id(&self, repo_id: &str, checkpoint_ref: &str) -> Result<String>;
    fn artefact_exists(&self, repo_id: &str, artefact_id: &str) -> Result<bool>;
}
