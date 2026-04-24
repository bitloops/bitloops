use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::subagents::extract_agent_id_from_text;
use super::{
    TranscriptLine, calculate_token_usage, calculate_total_token_usage,
    derive_tool_event_observations, extract_all_modified_files, extract_last_user_prompt,
    extract_modified_files, extract_spawned_agent_ids, find_checkpoint_uuid, parse_transcript,
    serialize_transcript, truncate_at_uuid,
};
use crate::host::interactions::tool_events::TranscriptToolEventObservation;

mod parsing;
mod subagents;
mod token_usage;
mod tool_events;

fn parse_lines(data: &str) -> Vec<TranscriptLine> {
    parse_transcript(data.as_bytes()).expect("failed to parse transcript lines")
}

fn write_jsonl_file(path: &Path, lines: &[String]) {
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    fs::write(path, body).expect("failed to write jsonl file");
}

fn make_assistant_tool_line(uuid: &str, tool_id: &str, name: &str, input: Value) -> String {
    serde_json::to_string(&json!({
        "type":"assistant",
        "uuid":uuid,
        "message":{
            "content":[{
                "type":"tool_use",
                "id":tool_id,
                "name":name,
                "input":input
            }]
        }
    }))
    .expect("assistant line must serialize")
}

fn make_write_tool_line(uuid: &str, file_path: &str) -> String {
    make_assistant_tool_line(
        uuid,
        &format!("toolu_{uuid}"),
        "Write",
        json!({"file_path": file_path}),
    )
}

fn make_edit_tool_line(uuid: &str, file_path: &str) -> String {
    make_assistant_tool_line(
        uuid,
        &format!("toolu_{uuid}"),
        "Edit",
        json!({"file_path": file_path}),
    )
}

fn make_task_tool_use_line(uuid: &str, tool_use_id: &str) -> String {
    make_assistant_tool_line(uuid, tool_use_id, "Task", json!({"prompt":"do something"}))
}

fn make_task_result_line(uuid: &str, tool_use_id: &str, agent_id: &str) -> String {
    serde_json::to_string(&json!({
        "type":"user",
        "uuid":uuid,
        "message":{
            "content":[{
                "type":"tool_result",
                "tool_use_id":tool_use_id,
                "content":format!("agentId: {agent_id}")
            }]
        }
    }))
    .expect("task result line must serialize")
}

fn contains_all(paths: &[String], expected: &[&str]) -> bool {
    let set = paths.iter().cloned().collect::<HashSet<_>>();
    expected.iter().all(|path| set.contains(*path))
}
