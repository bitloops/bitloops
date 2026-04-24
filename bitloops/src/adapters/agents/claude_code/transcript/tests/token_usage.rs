use serde_json::json;
use tempfile::tempdir;

use super::*;

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_BasicMessages() {
    let transcript = vec![
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-1".to_string(),
            message: json!({
                "id":"msg_001",
                "usage":{
                    "input_tokens":10,
                    "cache_creation_input_tokens":100,
                    "cache_read_input_tokens":50,
                    "output_tokens":20
                }
            }),
        },
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-2".to_string(),
            message: json!({
                "id":"msg_002",
                "usage":{
                    "input_tokens":5,
                    "cache_creation_input_tokens":200,
                    "cache_read_input_tokens":0,
                    "output_tokens":30
                }
            }),
        },
    ];

    let usage = calculate_token_usage(&transcript);
    assert_eq!(usage.api_call_count, 2);
    assert_eq!(usage.input_tokens, 15);
    assert_eq!(usage.cache_creation_tokens, 300);
    assert_eq!(usage.cache_read_tokens, 50);
    assert_eq!(usage.output_tokens, 50);
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_StreamingDeduplication() {
    let transcript = vec![
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-1".to_string(),
            message: json!({
                "id":"msg_001",
                "usage":{
                    "input_tokens":10,
                    "cache_creation_input_tokens":100,
                    "cache_read_input_tokens":50,
                    "output_tokens":1
                }
            }),
        },
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-2".to_string(),
            message: json!({
                "id":"msg_001",
                "usage":{
                    "input_tokens":10,
                    "cache_creation_input_tokens":100,
                    "cache_read_input_tokens":50,
                    "output_tokens":5
                }
            }),
        },
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-3".to_string(),
            message: json!({
                "id":"msg_001",
                "usage":{
                    "input_tokens":10,
                    "cache_creation_input_tokens":100,
                    "cache_read_input_tokens":50,
                    "output_tokens":20
                }
            }),
        },
    ];

    let usage = calculate_token_usage(&transcript);
    assert_eq!(usage.api_call_count, 1);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(usage.input_tokens, 10);
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_IgnoresUserMessages() {
    let transcript = vec![
        TranscriptLine {
            r#type: "user".to_string(),
            uuid: "user-1".to_string(),
            message: json!({"content":"hello"}),
        },
        TranscriptLine {
            r#type: "assistant".to_string(),
            uuid: "asst-1".to_string(),
            message: json!({
                "id":"msg_001",
                "usage":{
                    "input_tokens":10,
                    "cache_creation_input_tokens":100,
                    "cache_read_input_tokens":0,
                    "output_tokens":20
                }
            }),
        },
    ];

    let usage = calculate_token_usage(&transcript);
    assert_eq!(usage.api_call_count, 1);
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_EmptyTranscript() {
    let usage = calculate_token_usage(&[]);
    assert_eq!(usage.api_call_count, 0);
    assert_eq!(usage.input_tokens, 0);
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTotalTokenUsage_PerCheckpoint() {
    let dir = tempdir().expect("failed to create temp dir");
    let transcript_path = dir.path().join("transcript.jsonl");

    let content = concat!(
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"first prompt\"}}\n",
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50}}}\n",
        "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"content\":\"second prompt\"}}\n",
        "{\"type\":\"assistant\",\"uuid\":\"a2\",\"message\":{\"id\":\"m2\",\"usage\":{\"input_tokens\":200,\"output_tokens\":100}}}\n",
        "{\"type\":\"user\",\"uuid\":\"u3\",\"message\":{\"content\":\"third prompt\"}}\n",
        "{\"type\":\"assistant\",\"uuid\":\"a3\",\"message\":{\"id\":\"m3\",\"usage\":{\"input_tokens\":300,\"output_tokens\":150}}}\n"
    );
    std::fs::write(&transcript_path, content).expect("failed to write transcript");

    let usage1 =
        calculate_total_token_usage(transcript_path.to_str().expect("path must be utf-8"), 0, "")
            .expect("usage calculation should succeed");
    assert_eq!(usage1.input_tokens, 600);
    assert_eq!(usage1.output_tokens, 300);
    assert_eq!(usage1.api_call_count, 3);

    let usage2 =
        calculate_total_token_usage(transcript_path.to_str().expect("path must be utf-8"), 2, "")
            .expect("usage calculation should succeed");
    assert_eq!(usage2.input_tokens, 500);
    assert_eq!(usage2.output_tokens, 250);
    assert_eq!(usage2.api_call_count, 2);

    let usage3 =
        calculate_total_token_usage(transcript_path.to_str().expect("path must be utf-8"), 4, "")
            .expect("usage calculation should succeed");
    assert_eq!(usage3.input_tokens, 300);
    assert_eq!(usage3.output_tokens, 150);
    assert_eq!(usage3.api_call_count, 1);
}
