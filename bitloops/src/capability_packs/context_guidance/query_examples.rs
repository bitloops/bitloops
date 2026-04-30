use crate::host::capability_host::QueryExample;

use super::descriptor::CONTEXT_GUIDANCE_CAPABILITY_ID;

pub static CONTEXT_GUIDANCE_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
        name: "context_guidance.selected_history_guidance",
        query: "selectArtefacts(path:\"src/lib.rs\")->contextGuidance()",
        description: "List stored context guidance facts for selected artefacts",
    },
    QueryExample {
        capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
        name: "context_guidance.rejected_approaches",
        query: "selectArtefacts(path:\"src/lib.rs\")->contextGuidance(category:\"DECISION\", kind:\"rejected_approach\")",
        description: "List rejected approaches distilled from captured history",
    },
    QueryExample {
        capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
        name: "context_guidance.architectural_decisions_for_file",
        query: "selectArtefacts(path:\"src/lib.rs\")->contextGuidance(category:\"DECISION\", kind:\"architectural_boundary\")",
        description: "Show durable architectural decisions that should guide future edits to a file",
    },
];

#[cfg(test)]
mod tests {
    use super::CONTEXT_GUIDANCE_QUERY_EXAMPLES;

    #[test]
    fn context_guidance_query_examples_use_supported_slim_stage_syntax() {
        for example in CONTEXT_GUIDANCE_QUERY_EXAMPLES {
            assert!(
                example.query.contains("selectArtefacts("),
                "example `{}` must use selectArtefacts: {}",
                example.name,
                example.query
            );
            assert!(
                example.query.contains("contextGuidance("),
                "example `{}` must use the supported contextGuidance stage: {}",
                example.name,
                example.query
            );
            assert!(
                !example.query.contains("context_guidance("),
                "example `{}` uses unsupported snake_case stage syntax: {}",
                example.name,
                example.query
            );
        }
    }
}
