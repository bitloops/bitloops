use crate::host::devql::capability_host::QueryExample;

pub static KNOWLEDGE_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: "knowledge",
        name: "List knowledge entries",
        query: "repo(\"my-repo\") -> knowledge()",
        description: "List repository-scoped knowledge items",
    },
    QueryExample {
        capability_id: "knowledge",
        name: "List knowledge with asOf",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> knowledge()",
        description: "List knowledge items for a repository and revision context",
    },
    QueryExample {
        capability_id: "knowledge",
        name: "Select knowledge fields",
        query: "repo(\"my-repo\") -> knowledge() -> select(id, title, source_kind)",
        description: "Return selected fields from the knowledge stage",
    },
];
