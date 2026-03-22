use super::*;

#[test]
#[allow(non_snake_case)]
fn TestValidateSessionID() {
    let tests = vec![
        (
            "valid session ID with date prefix and uuid",
            "2026-01-25-f736da47-b2ca-4f86-bb32-a1bbe582e464",
            false,
            "",
        ),
        (
            "valid session ID with uuid only",
            "f736da47-b2ca-4f86-bb32-a1bbe582e464",
            false,
            "",
        ),
        (
            "valid session ID with special characters",
            "session-2026.01.25_test@123",
            false,
            "",
        ),
        ("empty session ID", "", true, "session ID cannot be empty"),
        (
            "session ID with forward slash",
            "session/123",
            true,
            "contains path separators",
        ),
        (
            "session ID with backslash",
            "session\\123",
            true,
            "contains path separators",
        ),
        (
            "path traversal attempt",
            "../../etc/passwd",
            true,
            "contains path separators",
        ),
        (
            "absolute unix path",
            "/etc/passwd",
            true,
            "contains path separators",
        ),
        (
            "absolute windows path",
            "C:\\Windows\\System32",
            true,
            "contains path separators",
        ),
    ];

    for (name, session_id, want_err, err_msg) in tests {
        let err = validate_session_id(session_id).err();
        if want_err {
            assert!(
                err.is_some(),
                "case {name}: ValidateSessionID({session_id:?}) expected error containing {err_msg:?}, got nil"
            );
            let err_str = err.unwrap().to_string();
            assert!(
                err_str.contains(err_msg),
                "case {name}: ValidateSessionID({session_id:?}) error = {err_str:?}, want error containing {err_msg:?}"
            );
        } else {
            assert!(
                err.is_none(),
                "case {name}: ValidateSessionID({session_id:?}) unexpected error: {:?}",
                err.unwrap()
            );
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn TestValidateToolUseID() {
    let tests = vec![
        (
            "valid uuid format",
            "f736da47-b2ca-4f86-bb32-a1bbe582e464",
            false,
            "",
        ),
        (
            "valid anthropic tool use id format",
            "toolu_abc123def456",
            false,
            "",
        ),
        ("valid alphanumeric only", "abc123DEF456", false, ""),
        (
            "valid with mixed underscores and hyphens",
            "tool_use-id-123",
            false,
            "",
        ),
        ("empty tool use ID is allowed", "", false, ""),
        (
            "path traversal attempt",
            "../../../etc/passwd",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "forward slash",
            "tool/use",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "backslash",
            "tool\\use",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "space in ID",
            "tool use id",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "dot in ID",
            "tool.use.id",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "special characters",
            "tool@use!id",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
        (
            "null byte",
            "tool\0use",
            true,
            "must be alphanumeric with underscores/hyphens only",
        ),
    ];

    for (name, tool_use_id, want_err, err_msg) in tests {
        let err = validate_tool_use_id(tool_use_id).err();
        if want_err {
            assert!(
                err.is_some(),
                "case {name}: ValidateToolUseID({tool_use_id:?}) expected error containing {err_msg:?}, got nil"
            );
            let err_str = err.unwrap().to_string();
            assert!(
                err_str.contains(err_msg),
                "case {name}: ValidateToolUseID({tool_use_id:?}) error = {err_str:?}, want error containing {err_msg:?}"
            );
        } else {
            assert!(
                err.is_none(),
                "case {name}: ValidateToolUseID({tool_use_id:?}) unexpected error: {:?}",
                err.unwrap()
            );
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn TestValidateAgentID() {
    let tests = vec![
        ("valid agent ID", "agent-test-123", false),
        (
            "valid uuid format",
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
            false,
        ),
        ("empty is allowed", "", false),
        ("slash rejected", "agent/test", true),
        ("dot rejected", "agent.test", true),
    ];

    for (name, agent_id, want_err) in tests {
        let err = validate_agent_id(agent_id).err();
        assert_eq!(
            err.is_some(),
            want_err,
            "case {name}: ValidateAgentID({agent_id:?}) error = {:?}, wantErr {want_err}",
            err
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestValidateAgentSessionID() {
    let tests = vec![
        ("valid uuid", "a1b2c3d4-e5f6-7890-abcd-ef1234567890", false),
        ("test session id", "test-session-1", false),
        ("alphanumeric", "session123", false),
        ("with underscores", "test_session_1", false),
        ("empty rejected", "", true),
        ("path traversal", "../../../etc/passwd", true),
        ("forward slash", "session/test", true),
        ("dot rejected", "session.test", true),
        ("space rejected", "session test", true),
    ];

    for (name, id, want_err) in tests {
        let err = validate_agent_session_id(id).err();
        assert_eq!(
            err.is_some(),
            want_err,
            "case {name}: ValidateAgentSessionID({id:?}) error = {:?}, wantErr {want_err}",
            err
        );
    }
}
