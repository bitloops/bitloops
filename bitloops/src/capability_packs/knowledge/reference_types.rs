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
    Commit {
        sha: String,
    },
    KnowledgeItem {
        knowledge_item_id: String,
        target_knowledge_item_version_id: Option<String>,
    },
    Checkpoint {
        checkpoint_id: String,
    },
    Artefact {
        artefact_id: String,
    },
}
