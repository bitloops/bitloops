use anyhow::Result;

use super::models::{KnowledgeItemRow, KnowledgeRelationAssertionRow, KnowledgeSourceRow};

pub trait KnowledgeRelationalRepository: Send + Sync {
    fn initialise_schema(&self) -> Result<()>;

    fn persist_ingestion(&self, source: &KnowledgeSourceRow, item: &KnowledgeItemRow)
    -> Result<()>;

    fn insert_relation_assertion(&self, relation: &KnowledgeRelationAssertionRow) -> Result<()>;

    fn find_item(&self, repo_id: &str, source_id: &str) -> Result<Option<KnowledgeItemRow>>;

    fn find_item_by_id(
        &self,
        repo_id: &str,
        knowledge_item_id: &str,
    ) -> Result<Option<KnowledgeItemRow>>;

    fn find_source_by_id(&self, knowledge_source_id: &str) -> Result<Option<KnowledgeSourceRow>>;

    fn list_items_for_repo(&self, repo_id: &str, limit: usize) -> Result<Vec<KnowledgeItemRow>>;
}
