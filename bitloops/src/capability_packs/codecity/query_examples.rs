use crate::host::capability_host::QueryExample;

use super::types::CODECITY_CAPABILITY_ID;

pub static CODECITY_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: CODECITY_CAPABILITY_ID,
        name: "CodeCity world",
        query: "repo(\"my-repo\") -> codecity_world()",
        description: "Build the renderer-ready CodeCity world with architecture-aware boundaries and zones.",
    },
    QueryExample {
        capability_id: CODECITY_CAPABILITY_ID,
        name: "CodeCity architecture",
        query: "repo(\"my-repo\") -> codecity_architecture()",
        description: "Inspect detected boundaries, macro topology, and architecture classifier scores.",
    },
    QueryExample {
        capability_id: CODECITY_CAPABILITY_ID,
        name: "CodeCity boundaries",
        query: "repo(\"my-repo\") -> codecity_boundaries()",
        description: "Inspect the deterministic CodeCity boundary assignments before classification.",
    },
];
