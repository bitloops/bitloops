use super::registry::AgentRegistry;
use super::*;
use crate::adapters::agents::gemini::agent::GeminiCliAgent;
use crate::adapters::agents::gemini::transcript::{GeminiMessage, GeminiTranscript};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct DispatchAgent {
    calls: Arc<AtomicUsize>,
}

impl Agent for DispatchAgent {
    fn name(&self) -> String {
        "dispatch-agent".to_string()
    }

    fn agent_type(&self) -> String {
        "dispatch-agent".to_string()
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = b"dispatch:".to_vec();
        result.extend_from_slice(&chunks.concat());
        Ok(result)
    }
}

#[test]
#[allow(non_snake_case)]
fn TestChunkJSONL_SmallContent() {
    let content = br#"{"type":"human","message":"hello"}
{"type":"assistant","message":"hi"}"#;

    let chunks = chunk_jsonl(content, MAX_CHUNK_SIZE).expect("chunk_jsonl should not error");
    assert_eq!(chunks.len(), 1, "expected one chunk for small content");
    assert_eq!(chunks[0], content, "chunk content mismatch");
}

#[test]
#[allow(non_snake_case)]
fn TestChunkJSONL_LargeContent() {
    let line_content = format!(r#"{{"type":"human","message":"{}"}}"#, "x".repeat(1000));
    let lines_needed = (MAX_CHUNK_SIZE / line_content.len()) + 100;
    let lines: Vec<String> = (0..lines_needed).map(|_| line_content.clone()).collect();
    let content = lines.join("\n").into_bytes();

    let chunks = chunk_jsonl(&content, MAX_CHUNK_SIZE).expect("chunk_jsonl should not error");
    assert!(
        chunks.len() >= 2,
        "expected at least 2 chunks for large content"
    );

    for (idx, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.len() <= MAX_CHUNK_SIZE,
            "chunk {idx} exceeds max chunk size"
        );
    }

    let reassembled = reassemble_jsonl(&chunks);
    assert_eq!(
        reassembled, content,
        "reassembled content must match original"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript_SmallContent_NoAgent() {
    let content = br#"{"type":"human","message":"hello"}"#;

    let registry = AgentRegistry::new(vec![]);
    let chunks =
        chunk_transcript(content, "", &registry).expect("chunk_transcript should not error");
    assert_eq!(chunks.len(), 1, "expected one chunk");
}

#[test]
#[allow(non_snake_case)]
fn TestChunkFileName() {
    let cases = [
        ("full.jsonl", 0, "full.jsonl"),
        ("full.jsonl", 1, "full.jsonl.001"),
        ("full.jsonl", 2, "full.jsonl.002"),
        ("full.jsonl", 10, "full.jsonl.010"),
        ("full.jsonl", 100, "full.jsonl.100"),
    ];

    for (base_name, index, expected) in cases {
        let result = chunk_file_name(base_name, index);
        assert_eq!(
            result, expected,
            "chunk_file_name({base_name}, {index}) should match expected"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestParseChunkIndex() {
    let cases = [
        ("full.jsonl", "full.jsonl", 0),
        ("full.jsonl.001", "full.jsonl", 1),
        ("full.jsonl.002", "full.jsonl", 2),
        ("full.jsonl.010", "full.jsonl", 10),
        ("full.jsonl.100", "full.jsonl", 100),
        ("other.txt", "full.jsonl", -1),
        ("full.jsonl.abc", "full.jsonl", -1),
    ];

    for (filename, base_name, expected) in cases {
        let result = parse_chunk_index(filename, base_name);
        assert_eq!(
            result, expected,
            "parse_chunk_index({filename}, {base_name}) should match expected"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestSortChunkFiles() {
    let files = vec![
        "full.jsonl.003".to_string(),
        "full.jsonl.001".to_string(),
        "full.jsonl".to_string(),
        "full.jsonl.002".to_string(),
    ];
    let expected = vec![
        "full.jsonl".to_string(),
        "full.jsonl.001".to_string(),
        "full.jsonl.002".to_string(),
        "full.jsonl.003".to_string(),
    ];

    let sorted = sort_chunk_files(&files, "full.jsonl");
    assert_eq!(
        sorted, expected,
        "sorted chunk files must be in chunk order"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleJSONL_SingleChunk() {
    let content = br#"{"type":"human","message":"hello"}"#.to_vec();
    let chunks = vec![content.clone()];

    let result = reassemble_jsonl(&chunks);
    assert_eq!(
        result, content,
        "single chunk reassembly should be identity"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleTranscript_EmptyChunks() {
    let registry = AgentRegistry::new(vec![]);
    let result =
        reassemble_transcript(Vec::new(), "", &registry).expect("reassemble should not error");
    assert!(result.is_none(), "expected None for empty chunks");
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleTranscript_UsesAgentSpecificImplementation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = AgentRegistry::new(vec![Box::new(DispatchAgent {
        calls: calls.clone(),
    })]);

    let chunks = vec![b"one".to_vec(), b"two".to_vec()];
    let result = reassemble_transcript(chunks, "dispatch-agent", &registry)
        .expect("reassemble should not error")
        .expect("expected reassembled transcript");

    assert_eq!(result, b"dispatch:onetwo".to_vec());
    assert_eq!(calls.load(Ordering::SeqCst), 1, "agent path should be used");
}

#[test]
#[allow(non_snake_case)]
fn TestGeminiCliAgentChunkAndReassembleTranscript() {
    let agent = GeminiCliAgent;
    let transcript = GeminiTranscript {
        messages: vec![
            GeminiMessage {
                id: "msg-1".to_string(),
                r#type: "user".to_string(),
                content: "hello".to_string(),
                tool_calls: Vec::new(),
                ..Default::default()
            },
            GeminiMessage {
                id: "msg-2".to_string(),
                r#type: "gemini".to_string(),
                content: "a".repeat(120),
                tool_calls: Vec::new(),
                ..Default::default()
            },
            GeminiMessage {
                id: "msg-3".to_string(),
                r#type: "user".to_string(),
                content: "follow up".to_string(),
                tool_calls: Vec::new(),
                ..Default::default()
            },
        ],
    };

    let content = serde_json::to_vec(&transcript).expect("marshal transcript");
    let chunks = agent
        .chunk_transcript(&content, 180)
        .expect("chunking should succeed");
    assert!(
        chunks.len() >= 2,
        "expected Gemini transcript to split into multiple chunks"
    );

    let reassembled = agent
        .reassemble_transcript(&chunks)
        .expect("reassembly should succeed");
    let reassembled_transcript: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("parse reassembled transcript");

    assert_eq!(
        serde_json::to_value(&reassembled_transcript).expect("marshal reassembled transcript"),
        serde_json::to_value(&transcript).expect("marshal original transcript"),
        "reassembled Gemini transcript should match the original structure"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleJSONL_MultipleChunks() {
    let chunk1 = br#"{"line":1}"#.to_vec();
    let chunk2 = br#"{"line":2}"#.to_vec();
    let chunks = vec![chunk1, chunk2];

    let result = reassemble_jsonl(&chunks);
    let expected = br#"{"line":1}
{"line":2}"#
        .to_vec();
    assert_eq!(result, expected, "reassembled JSONL must include newline");
}

#[test]
#[allow(non_snake_case)]
fn TestChunkJSONL_OversizedLine() {
    let max_size = 100;
    let oversized_line = format!(r#"{{"type":"human","message":"{}"}}"#, "x".repeat(max_size));
    let content = oversized_line.into_bytes();

    let err = chunk_jsonl(&content, max_size).expect_err("expected oversized line error");
    assert!(
        err.to_string().contains("exceeds maximum chunk size"),
        "expected size error message"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestChunkJSONL_OversizedLineInMiddle() {
    let max_size = 100;
    let normal_line = r#"{"type":"human","message":"short"}"#;
    let oversized_line = format!(
        r#"{{"type":"assistant","message":"{}"}}"#,
        "x".repeat(max_size)
    );
    let content = format!("{normal_line}\n{oversized_line}\n{normal_line}").into_bytes();

    let err = chunk_jsonl(&content, max_size).expect_err("expected oversized line error in middle");
    assert!(
        err.to_string().contains("line 2"),
        "expected error to mention line number"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestDetectAgentTypeFromContent() {
    let cases = [
        (
            "Gemini JSON",
            br#"{"messages":[{"type":"user","content":"hi"}]}"#.as_slice(),
            AGENT_TYPE_GEMINI,
        ),
        (
            "JSONL",
            br#"{"type":"human","message":"hi"}"#.as_slice(),
            "",
        ),
        ("Empty messages array", br#"{"messages":[]}"#.as_slice(), ""),
        ("Invalid JSON", br#"not json"#.as_slice(), ""),
    ];

    for (name, content, expected) in cases {
        let result = detect_agent_type_from_content(content);
        assert_eq!(result, expected, "case {name} mismatch");
    }
}
