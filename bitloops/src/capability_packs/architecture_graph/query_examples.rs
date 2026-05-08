use crate::host::capability_host::QueryExample;

use super::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;

pub static ARCHITECTURE_GRAPH_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
        name: "Architecture graph",
        query: "repo(\"my-repo\") -> architecture_graph()",
        description: "Read the effective architecture graph for the current project scope.",
    },
    QueryExample {
        capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
        name: "Architecture entry points",
        query: "repo(\"my-repo\") -> architecture_entry_points(kind: \"main\")",
        description: "Inspect effective entry points resolved from language evidence and assertions.",
    },
    QueryExample {
        capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
        name: "Architecture flows",
        query: "repo(\"my-repo\") -> architecture_flows(entryPointId: \"...\")",
        description: "Inspect flows triggered by an entry point, the nodes they traverse, and ordered module steps.",
    },
];
