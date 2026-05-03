use crate::host::capability_host::QueryExample;

use super::types::NAVIGATION_CONTEXT_CAPABILITY_ID;

pub static NAVIGATION_CONTEXT_QUERY_EXAMPLES: &[QueryExample] = &[QueryExample {
    capability_id: NAVIGATION_CONTEXT_CAPABILITY_ID,
    name: "navigation_context_status",
    query: r#"
query {
  health {
    status
  }
}
"#,
    description: "Navigation context currently materialises hashed primitives and freshness state through the current-state consumer; typed query fields are a follow-up surface.",
}];
