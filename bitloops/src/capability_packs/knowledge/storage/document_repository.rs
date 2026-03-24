use anyhow::Result;

use super::models::KnowledgeDocumentVersionRow;

pub trait KnowledgeDocumentRepository: Send + Sync {
    fn initialise_schema(&self) -> Result<()>;

    fn has_knowledge_item_version(
        &self,
        knowledge_item_id: &str,
        content_hash: &str,
    ) -> Result<Option<String>>;

    fn insert_knowledge_item_version(&self, row: &KnowledgeDocumentVersionRow) -> Result<()>;

    fn delete_knowledge_item_version(&self, knowledge_item_version_id: &str) -> Result<()>;

    fn find_knowledge_item_version(
        &self,
        knowledge_item_version_id: &str,
    ) -> Result<Option<KnowledgeDocumentVersionRow>>;

    fn list_versions_for_item(
        &self,
        knowledge_item_id: &str,
    ) -> Result<Vec<KnowledgeDocumentVersionRow>>;
}
