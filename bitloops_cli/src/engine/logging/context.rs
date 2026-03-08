use super::logger::Attr;
use super::logger::string_attr;

#[derive(Clone, Debug, Default)]
pub struct LogContext {
    session_id: Option<String>,
    parent_session_id: Option<String>,
    tool_call_id: Option<String>,
    component: Option<String>,
    agent: Option<String>,
}

pub fn background() -> LogContext {
    LogContext::default()
}

pub fn with_session(mut ctx: LogContext, session_id: &str) -> LogContext {
    let next = session_id.to_string();
    if let Some(existing) = &ctx.session_id
        && !existing.is_empty()
        && existing != &next
    {
        ctx.parent_session_id = Some(existing.clone());
    }
    ctx.session_id = Some(next);
    ctx
}

pub fn with_parent_session(mut ctx: LogContext, parent_session_id: &str) -> LogContext {
    ctx.parent_session_id = Some(parent_session_id.to_string());
    ctx
}

pub fn with_tool_call(mut ctx: LogContext, tool_call_id: &str) -> LogContext {
    ctx.tool_call_id = Some(tool_call_id.to_string());
    ctx
}

pub fn with_component(mut ctx: LogContext, component: &str) -> LogContext {
    ctx.component = Some(component.to_string());
    ctx
}

pub fn with_agent(mut ctx: LogContext, agent_name: &str) -> LogContext {
    ctx.agent = Some(agent_name.to_string());
    ctx
}

pub fn session_id_from_context(ctx: &LogContext) -> String {
    ctx.session_id.clone().unwrap_or_default()
}

pub fn parent_session_id_from_context(ctx: &LogContext) -> String {
    ctx.parent_session_id.clone().unwrap_or_default()
}

pub fn tool_call_id_from_context(ctx: &LogContext) -> String {
    ctx.tool_call_id.clone().unwrap_or_default()
}

pub fn component_from_context(ctx: &LogContext) -> String {
    ctx.component.clone().unwrap_or_default()
}

pub fn agent_from_context(ctx: &LogContext) -> String {
    ctx.agent.clone().unwrap_or_default()
}

pub fn attrs_from_context(ctx: &LogContext, global_session_id: &str) -> Vec<Attr> {
    let mut attrs = Vec::new();

    if global_session_id.is_empty() {
        let session_id = session_id_from_context(ctx);
        if !session_id.is_empty() {
            attrs.push(string_attr("session_id", &session_id));
        }
    }

    let parent_session_id = parent_session_id_from_context(ctx);
    if !parent_session_id.is_empty() {
        attrs.push(string_attr("parent_session_id", &parent_session_id));
    }

    let tool_call_id = tool_call_id_from_context(ctx);
    if !tool_call_id.is_empty() {
        attrs.push(string_attr("tool_call_id", &tool_call_id));
    }

    let component = component_from_context(ctx);
    if !component.is_empty() {
        attrs.push(string_attr("component", &component));
    }

    let agent = agent_from_context(ctx);
    if !agent.is_empty() {
        attrs.push(string_attr("agent", &agent));
    }

    attrs
}
