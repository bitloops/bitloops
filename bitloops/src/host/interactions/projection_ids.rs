pub(crate) fn scope_tool_projection_id(
    turn_id: Option<&str>,
    session_id: &str,
    tool_use_id: &str,
) -> String {
    if tool_use_id.trim().is_empty() {
        return String::new();
    }

    if let Some(turn_id) = turn_id.map(str::trim).filter(|turn_id| !turn_id.is_empty()) {
        return prefix_projection_scope(turn_id, tool_use_id);
    }

    let session_id = session_id.trim();
    if !session_id.is_empty() {
        return prefix_projection_scope(session_id, tool_use_id);
    }

    tool_use_id.to_string()
}

fn prefix_projection_scope(scope: &str, tool_use_id: &str) -> String {
    if tool_use_id
        .trim()
        .strip_prefix(scope)
        .is_some_and(|rest| rest.starts_with(':'))
    {
        return tool_use_id.to_string();
    }

    format!("{scope}:{tool_use_id}")
}
