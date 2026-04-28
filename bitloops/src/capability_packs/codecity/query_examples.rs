use crate::host::capability_host::QueryExample;

use super::types::CODECITY_CAPABILITY_ID;

pub static CODECITY_QUERY_EXAMPLES: &[QueryExample] = &[QueryExample {
    capability_id: CODECITY_CAPABILITY_ID,
    name: "CodeCity world",
    query: "repo(\"my-repo\") -> codecity_world()",
    description: "Build a current-scope CodeCity world from DevQL current artefacts and dependency edges.",
}];
