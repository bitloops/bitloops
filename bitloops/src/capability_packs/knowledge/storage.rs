pub mod blob_payloads;
pub mod document_repository;
pub mod duckdb_documents;
pub mod models;
pub mod relational_repository;
pub mod sqlite_relational;

pub use blob_payloads::BlobKnowledgePayloadStore;
pub use document_repository::KnowledgeDocumentRepository;
pub use duckdb_documents::DuckdbKnowledgeDocumentStore;
pub use models::{
    KnowledgeDocumentVersionRow, KnowledgeItemRow, KnowledgePayloadRef,
    KnowledgeRelationAssertionRow, KnowledgeSourceRow, content_hash, ensure_parent_dir,
    knowledge_item_id, knowledge_item_version_id, knowledge_payload_key, knowledge_source_id,
    relation_assertion_id, serialize_payload,
};
pub use relational_repository::KnowledgeRelationalRepository;
pub use sqlite_relational::SqliteKnowledgeRelationalRepository;
