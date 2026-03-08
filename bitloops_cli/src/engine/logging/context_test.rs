use super::*;
use std::collections::HashMap;

const TEST_COMPONENT: &str = "hooks";
const TEST_AGENT: &str = "claude-code";

#[test]
#[allow(non_snake_case)]
fn TestWithSession() {
    let ctx = background();
    let session_id = "2025-01-15-test-session";

    let ctx = with_session(ctx, session_id);
    assert_eq!(
        session_id_from_context(&ctx),
        session_id,
        "session_id_from_context should return the value set by with_session"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWithSession_SetsParentFromExisting() {
    let ctx = background();
    let parent_session_id = "2025-01-15-parent-session";
    let child_session_id = "2025-01-15-child-session";

    let ctx = with_session(ctx, parent_session_id);
    let ctx = with_session(ctx, child_session_id);

    assert_eq!(
        session_id_from_context(&ctx),
        child_session_id,
        "with_session should overwrite current session with child session"
    );
    assert_eq!(
        parent_session_id_from_context(&ctx),
        parent_session_id,
        "with_session should preserve previous session as parent session"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWithParentSession() {
    let ctx = background();
    let parent_session_id = "2025-01-15-explicit-parent";

    let ctx = with_parent_session(ctx, parent_session_id);
    assert_eq!(
        parent_session_id_from_context(&ctx),
        parent_session_id,
        "parent_session_id_from_context should return explicit parent session"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWithToolCall() {
    let ctx = background();
    let tool_call_id = "toolu_01ABC123XYZ";

    let ctx = with_tool_call(ctx, tool_call_id);
    assert_eq!(
        tool_call_id_from_context(&ctx),
        tool_call_id,
        "tool_call_id_from_context should return the value set by with_tool_call"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWithComponent() {
    let ctx = background();
    let ctx = with_component(ctx, TEST_COMPONENT);

    assert_eq!(
        component_from_context(&ctx),
        TEST_COMPONENT,
        "component_from_context should return the component value"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWithAgent() {
    let ctx = background();
    let ctx = with_agent(ctx, TEST_AGENT);

    assert_eq!(
        agent_from_context(&ctx),
        TEST_AGENT,
        "agent_from_context should return the agent value"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestContextValues_Empty() {
    let ctx = background();

    assert_eq!(
        session_id_from_context(&ctx),
        "",
        "session_id should be empty for background context"
    );
    assert_eq!(
        parent_session_id_from_context(&ctx),
        "",
        "parent_session_id should be empty for background context"
    );
    assert_eq!(
        tool_call_id_from_context(&ctx),
        "",
        "tool_call_id should be empty for background context"
    );
    assert_eq!(
        component_from_context(&ctx),
        "",
        "component should be empty for background context"
    );
    assert_eq!(
        agent_from_context(&ctx),
        "",
        "agent should be empty for background context"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestContextValues_Chaining() {
    let ctx = background();
    let ctx = with_session(ctx, "session-1");
    let ctx = with_tool_call(ctx, "tool-1");
    let ctx = with_component(ctx, TEST_COMPONENT);
    let ctx = with_agent(ctx, TEST_AGENT);

    assert_eq!(
        session_id_from_context(&ctx),
        "session-1",
        "session_id should survive context chaining"
    );
    assert_eq!(
        tool_call_id_from_context(&ctx),
        "tool-1",
        "tool_call_id should survive context chaining"
    );
    assert_eq!(
        component_from_context(&ctx),
        TEST_COMPONENT,
        "component should survive context chaining"
    );
    assert_eq!(
        agent_from_context(&ctx),
        TEST_AGENT,
        "agent should survive context chaining"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAttrsFromContext() {
    let ctx = background();
    let ctx = with_session(ctx, "session-123");
    let ctx = with_parent_session(ctx, "parent-456");
    let ctx = with_tool_call(ctx, "tool-789");
    let ctx = with_component(ctx, TEST_COMPONENT);
    let ctx = with_agent(ctx, TEST_AGENT);

    let attrs = attrs_from_context(&ctx, "");
    assert_eq!(
        attrs.len(),
        5,
        "attrs_from_context should return 5 attrs when all fields are set"
    );

    let attr_map: HashMap<String, String> = attrs
        .iter()
        .map(|attr| (attr.key.clone(), attr.as_string()))
        .collect();

    assert_eq!(
        attr_map.get("session_id"),
        Some(&"session-123".to_string()),
        "session_id attr mismatch"
    );
    assert_eq!(
        attr_map.get("parent_session_id"),
        Some(&"parent-456".to_string()),
        "parent_session_id attr mismatch"
    );
    assert_eq!(
        attr_map.get("tool_call_id"),
        Some(&"tool-789".to_string()),
        "tool_call_id attr mismatch"
    );
    assert_eq!(
        attr_map.get("component"),
        Some(&TEST_COMPONENT.to_string()),
        "component attr mismatch"
    );
    assert_eq!(
        attr_map.get("agent"),
        Some(&TEST_AGENT.to_string()),
        "agent attr mismatch"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAttrsFromContext_Partial() {
    let ctx = with_session(background(), "session-only");
    let attrs = attrs_from_context(&ctx, "");

    assert_eq!(
        attrs.len(),
        1,
        "attrs_from_context should return exactly one attr when only session is set"
    );

    if attrs.len() == 1 {
        assert_eq!(attrs[0].key, "session_id", "attr key should be session_id");
        assert_eq!(
            attrs[0].as_string(),
            "session-only",
            "session_id value mismatch"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestAttrsFromContext_SkipsSessionWhenGlobalSet() {
    let ctx = with_tool_call(with_session(background(), "context-session"), "tool-123");
    let attrs = attrs_from_context(&ctx, "global-session");

    assert_eq!(
        attrs.len(),
        1,
        "attrs_from_context should skip context session_id when global session_id is set"
    );

    if attrs.len() == 1 {
        assert_eq!(
            attrs[0].key, "tool_call_id",
            "attr key should be tool_call_id"
        );
        assert_eq!(
            attrs[0].as_string(),
            "tool-123",
            "tool_call_id value mismatch"
        );
    }
}
