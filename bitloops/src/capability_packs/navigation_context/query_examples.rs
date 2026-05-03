use crate::host::capability_host::QueryExample;

use super::types::NAVIGATION_CONTEXT_CAPABILITY_ID;

pub static NAVIGATION_CONTEXT_QUERY_EXAMPLES: &[QueryExample] = &[QueryExample {
    capability_id: NAVIGATION_CONTEXT_CAPABILITY_ID,
    name: "navigation_context_status",
    query: r#"
query {
  project(path: ".") {
    navigationContext(filter: { viewStatus: STALE }) {
      views {
        viewId
        label
        status
        staleReason
      }
    }
  }
}
"#,
    description: "List stale navigation context views and their dependency-change explanations for the current repository.",
}];
