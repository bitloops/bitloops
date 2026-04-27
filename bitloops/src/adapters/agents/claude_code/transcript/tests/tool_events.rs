use serde_json::json;

use super::*;

#[test]
fn derives_claude_tool_event_observations_from_transcript_fragment() {
    let fragment = concat!(
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/lib.rs\"}},",
        "{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"Bash\",\"input\":{\"command\":\"rg interaction_events src\"}}",
        "]}}\n",
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_2\",\"content\":\"found matches\"}",
        "]}}\n"
    );

    let observations =
        derive_tool_event_observations("turn-1", fragment).expect("derive observations");

    assert_eq!(
        observations,
        vec![
            TranscriptToolEventObservation::Invocation {
                tool_use_id: "toolu_1".to_string(),
                tool_name: "Read".to_string(),
                tool_input: json!({"file_path":"src/lib.rs"}),
            },
            TranscriptToolEventObservation::Invocation {
                tool_use_id: "toolu_2".to_string(),
                tool_name: "Bash".to_string(),
                tool_input: json!({"command":"rg interaction_events src"}),
            },
            TranscriptToolEventObservation::Result {
                tool_use_id: "toolu_2".to_string(),
                tool_name: "Bash".to_string(),
                tool_output: json!("found matches"),
            },
        ]
    );
}

#[test]
fn ignores_claude_subagent_task_observations() {
    let fragment = concat!(
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_use\",\"id\":\"toolu_task\",\"name\":\"Task\",\"input\":{\"prompt\":\"delegate\"}}",
        "]}}\n",
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_task\",\"content\":\"agentId: sub123\"}",
        "]}}\n"
    );

    let observations =
        derive_tool_event_observations("turn-1", fragment).expect("derive observations");
    assert!(observations.is_empty());
}

#[test]
fn idless_claude_tool_uses_after_subagent_tasks_receive_unique_fallback_ids() {
    let fragment = concat!(
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_use\",\"name\":\"Task\",\"input\":{\"prompt\":\"delegate\"}},",
        "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"src/lib.rs\"}}",
        "]}}\n",
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
        "{\"type\":\"tool_result\",\"tool_use_id\":\"turn-1:tool:0002\",\"content\":\"updated file\"}",
        "]}}\n"
    );

    let observations =
        derive_tool_event_observations("turn-1", fragment).expect("derive observations");
    assert_eq!(
        observations,
        vec![
            TranscriptToolEventObservation::Invocation {
                tool_use_id: "turn-1:tool:0002".to_string(),
                tool_name: "Edit".to_string(),
                tool_input: json!({"file_path":"src/lib.rs"}),
            },
            TranscriptToolEventObservation::Result {
                tool_use_id: "turn-1:tool:0002".to_string(),
                tool_name: "Edit".to_string(),
                tool_output: json!("updated file"),
            },
        ]
    );
}
