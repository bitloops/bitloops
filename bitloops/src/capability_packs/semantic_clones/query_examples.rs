use crate::host::capability_host::QueryExample;

use super::types::SEMANTIC_CLONES_CAPABILITY_ID;

pub static SEMANTIC_CLONES_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID,
        name: "semantic_clones.clones_with_artefacts",
        query: "repo(\"bitloops\")->artefacts(kind:\"symbol\")->clones(min_score:0.5)->limit(10)",
        description: "Relational clones() stage over symbol artefacts (requires ingest)",
    },
    QueryExample {
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID,
        name: "semantic_clones.clone_summary",
        query: "repo(\"bitloops\")->artefacts(kind:\"symbol\")->clones(min_score:0.5)->summary()",
        description: "Aggregated clone counts grouped by relation kind over the filtered clone result set",
    },
];
