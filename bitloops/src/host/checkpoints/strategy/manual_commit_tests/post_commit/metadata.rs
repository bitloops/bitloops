use super::*;

#[test]
pub(crate) fn extract_user_prompts_supports_nested_message_and_human_type() {
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create index.html"},{"type":"tool_result","tool_use_id":"x"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
{"type":"human","message":{"content":"Add styles"}}
not-json"#;

    let prompts = extract_user_prompts_from_jsonl(jsonl);
    assert_eq!(prompts, vec!["Create index.html", "Add styles"]);
}

#[test]
pub(crate) fn extract_summary_supports_nested_message_content() {
    let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first summary"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"final summary"},{"type":"tool_use","name":"Edit","input":{"file_path":"a.txt"}}]}}"#;

    let summary = extract_summary_from_jsonl(jsonl);
    assert_eq!(summary, "final summary");
}

#[test]
pub(crate) fn write_session_metadata_writes_prompt_and_summary_for_nested_claude_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let transcript_path = dir.path().join("transcript.jsonl");
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create test file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Created test file"}]}}"#;
    fs::write(&transcript_path, jsonl).unwrap();

    let written = write_session_metadata(
        dir.path(),
        "session-nested",
        &transcript_path.to_string_lossy(),
    )
    .unwrap();
    assert!(
        written.contains(&".bitloops/metadata/session-nested/prompt.txt".to_string()),
        "prompt.txt should be part of written metadata files: {written:?}"
    );
    assert!(
        written.contains(&".bitloops/metadata/session-nested/summary.txt".to_string()),
        "summary.txt should be part of written metadata files: {written:?}"
    );

    let prompt = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("prompt.txt"),
    )
    .unwrap();
    let summary = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("summary.txt"),
    )
    .unwrap();

    assert_eq!(prompt.trim(), "Create test file");
    assert_eq!(summary.trim(), "Created test file");
}
