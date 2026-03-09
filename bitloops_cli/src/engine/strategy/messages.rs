//! Message-formatting helpers for manual-commit checkpoints.

use serde::Deserialize;

pub const MAX_DESCRIPTION_LENGTH: usize = 60;

pub fn truncate_description(s: &str, max_len: usize) -> String {
    let len = s.chars().count();
    if len <= max_len {
        return s.to_string();
    }
    if max_len < 3 {
        return s.chars().take(max_len).collect();
    }
    let mut out: String = s.chars().take(max_len - 3).collect();
    out.push_str("...");
    out
}

fn format_subagent_message(
    verb: &str,
    agent_type: &str,
    description: &str,
    tool_use_id: &str,
) -> String {
    if agent_type.is_empty() && description.is_empty() {
        return format!("Task: {tool_use_id}");
    }

    let description = if description.is_empty() {
        String::new()
    } else {
        truncate_description(description, MAX_DESCRIPTION_LENGTH)
    };

    if !agent_type.is_empty() && !description.is_empty() {
        return format!("{verb} '{agent_type}' agent: {description} ({tool_use_id})");
    }
    if !agent_type.is_empty() {
        return format!("{verb} '{agent_type}' agent ({tool_use_id})");
    }

    format!("{verb} agent: {description} ({tool_use_id})")
}

pub fn format_subagent_end_message(
    agent_type: &str,
    description: &str,
    tool_use_id: &str,
) -> String {
    format_subagent_message("Completed", agent_type, description, tool_use_id)
}

pub fn format_incremental_message(todo_content: &str, sequence: u32, tool_use_id: &str) -> String {
    if todo_content.is_empty() {
        return format!("Checkpoint #{sequence}: {tool_use_id}");
    }
    let todo = truncate_description(todo_content, MAX_DESCRIPTION_LENGTH);
    format!("{todo} ({tool_use_id})")
}

pub fn format_incremental_subject(
    _incremental_type: &str,
    _subagent_type: &str,
    _task_description: &str,
    todo_content: &str,
    incremental_sequence: u32,
    short_tool_use_id: &str,
) -> String {
    format_incremental_message(todo_content, incremental_sequence, short_tool_use_id)
}

#[derive(Debug, Deserialize)]
struct TodoItem {
    #[serde(default)]
    content: String,
    #[serde(default)]
    status: String,
}

pub fn extract_last_completed_todo(todos_json: &[u8]) -> String {
    if todos_json.is_empty() {
        return String::new();
    }
    let Ok(todos) = serde_json::from_slice::<Vec<TodoItem>>(todos_json) else {
        return String::new();
    };
    let mut last_completed = String::new();
    for todo in todos {
        if todo.status == "completed" {
            last_completed = todo.content;
        }
    }
    last_completed
}

pub fn count_todos(todos_json: &[u8]) -> usize {
    if todos_json.is_empty() {
        return 0;
    }
    let Ok(todos) = serde_json::from_slice::<Vec<TodoItem>>(todos_json) else {
        return 0;
    };
    todos.len()
}

pub fn extract_in_progress_todo(todos_json: &[u8]) -> String {
    if todos_json.is_empty() {
        return String::new();
    }
    let Ok(todos) = serde_json::from_slice::<Vec<TodoItem>>(todos_json) else {
        return String::new();
    };
    if todos.is_empty() {
        return String::new();
    }

    for todo in &todos {
        if todo.status == "in_progress" {
            return todo.content.clone();
        }
    }
    for todo in &todos {
        if todo.status == "pending" {
            return todo.content.clone();
        }
    }
    let mut last_completed = String::new();
    for todo in &todos {
        if todo.status == "completed" {
            last_completed = todo.content.clone();
        }
    }
    if !last_completed.is_empty() {
        return last_completed;
    }
    todos[0].content.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_description() {
        let cases = [
            ("Short", 60usize, "Short"),
            ("123456", 6, "123456"),
            (
                "This is a very long description that exceeds the maximum length",
                30,
                "This is a very long descrip...",
            ),
            ("", 60, ""),
            ("Hello", 2, "He"),
        ];

        for (input, max_len, want) in cases {
            assert_eq!(truncate_description(input, max_len), want);
        }
    }

    #[test]
    fn test_format_subagent_end_message() {
        assert_eq!(
            format_subagent_end_message("dev", "Implement user authentication", "toolu_019t1c"),
            "Completed 'dev' agent: Implement user authentication (toolu_019t1c)"
        );
        assert_eq!(
            format_subagent_end_message("dev", "", "toolu_019t1c"),
            "Completed 'dev' agent (toolu_019t1c)"
        );
        assert_eq!(
            format_subagent_end_message("", "Implement user authentication", "toolu_019t1c"),
            "Completed agent: Implement user authentication (toolu_019t1c)"
        );
        assert_eq!(
            format_subagent_end_message("", "", "toolu_019t1c"),
            "Task: toolu_019t1c"
        );
    }

    #[test]
    fn test_format_incremental_message() {
        assert_eq!(
            format_incremental_message(
                "Set up Node.js project with package.json",
                1,
                "toolu_01CJhrr"
            ),
            "Set up Node.js project with package.json (toolu_01CJhrr)"
        );
        assert_eq!(
            format_incremental_message("", 3, "toolu_01CJhrr"),
            "Checkpoint #3: toolu_01CJhrr"
        );
        assert_eq!(
            format_incremental_message(
                "This is a very long todo item that describes in detail what needs to be done for this step of the implementation process",
                2,
                "toolu_01CJhrr"
            ),
            "This is a very long todo item that describes in detail wh... (toolu_01CJhrr)"
        );
    }

    #[test]
    fn test_extract_last_completed_todo() {
        let cases = [
            (
                r#"[{"content":"First task","status":"completed"},{"content":"Second task","status":"completed"},{"content":"Third task","status":"in_progress"}]"#,
                "Second task",
            ),
            (
                r#"[{"content":"First task","status":"completed"}]"#,
                "First task",
            ),
            (
                r#"[{"content":"First task","status":"completed"},{"content":"Second task","status":"completed"},{"content":"Third task","status":"completed"}]"#,
                "Third task",
            ),
            (
                r#"[{"content":"First task","status":"in_progress"},{"content":"Second task","status":"pending"}]"#,
                "",
            ),
            (r#"[]"#, ""),
            ("not valid json", ""),
            ("null", ""),
            (
                r#"[{"content":"Done 1","status":"completed"},{"content":"Pending 1","status":"pending"},{"content":"Done 2","status":"completed"},{"content":"Pending 2","status":"pending"}]"#,
                "Done 2",
            ),
        ];

        for (todos_json, want) in cases {
            assert_eq!(extract_last_completed_todo(todos_json.as_bytes()), want);
        }
    }

    #[test]
    fn test_count_todos() {
        let cases = [
            (
                r#"[{"content":"First task","status":"completed"},{"content":"Second task","status":"in_progress"},{"content":"Third task","status":"pending"}]"#,
                3usize,
            ),
            (r#"[{"content":"Only task","status":"pending"}]"#, 1),
            (r#"[]"#, 0),
            ("not valid json", 0),
            ("null", 0),
            (
                r#"[{"content":"Task 1","status":"pending"},{"content":"Task 2","status":"pending"},{"content":"Task 3","status":"pending"},{"content":"Task 4","status":"pending"},{"content":"Task 5","status":"pending"},{"content":"Task 6","status":"in_progress"}]"#,
                6,
            ),
        ];

        for (todos_json, want) in cases {
            assert_eq!(count_todos(todos_json.as_bytes()), want);
        }
    }

    #[test]
    fn test_format_incremental_subject() {
        assert_eq!(
            format_incremental_subject(
                "todo_write",
                "dev",
                "task description",
                "Set up Node.js project with package.json",
                1,
                "toolu_01CJhrr"
            ),
            "Set up Node.js project with package.json (toolu_01CJhrr)"
        );
        assert_eq!(
            format_incremental_subject(
                "todo_write",
                "dev",
                "task description",
                "",
                3,
                "toolu_01CJhrr"
            ),
            "Checkpoint #3: toolu_01CJhrr"
        );
    }

    #[test]
    fn test_extract_in_progress_todo() {
        assert_eq!(
            extract_in_progress_todo(
                r#"[{"content":"done","status":"completed"},{"content":"doing now","status":"in_progress"},{"content":"next","status":"pending"}]"#
                    .as_bytes()
            ),
            "doing now"
        );
        assert_eq!(
            extract_in_progress_todo(
                r#"[{"content":"done","status":"completed"},{"content":"next pending","status":"pending"}]"#
                    .as_bytes()
            ),
            "next pending"
        );
        assert_eq!(
            extract_in_progress_todo(
                r#"[{"content":"done-1","status":"completed"},{"content":"done-2","status":"completed"}]"#
                    .as_bytes()
            ),
            "done-2"
        );
        assert_eq!(extract_in_progress_todo("[]".as_bytes()), "");
        assert_eq!(extract_in_progress_todo("not valid".as_bytes()), "");
    }
}
