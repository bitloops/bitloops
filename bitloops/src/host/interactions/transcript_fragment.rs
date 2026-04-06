use serde_json::Value;

pub(crate) fn transcript_position_from_bytes(transcript_data: &[u8]) -> usize {
    if transcript_data.is_empty() {
        return 0;
    }
    if let Some(message_count) = structured_message_count(transcript_data) {
        return message_count;
    }
    String::from_utf8_lossy(transcript_data)
        .split_inclusive('\n')
        .count()
}

pub(crate) fn transcript_fragment_from_bytes(
    transcript_data: &[u8],
    start_offset: usize,
    end_offset: usize,
) -> String {
    if start_offset >= end_offset {
        return String::new();
    }
    if let Some(fragment) =
        structured_transcript_fragment_from_offsets(transcript_data, start_offset, end_offset)
    {
        return fragment;
    }

    let transcript_text = String::from_utf8_lossy(transcript_data);
    let lines: Vec<&str> = transcript_text.split_inclusive('\n').collect();
    if start_offset >= lines.len() {
        return String::new();
    }
    let bounded_end = end_offset.min(lines.len());
    lines[start_offset..bounded_end].concat()
}

pub(crate) fn read_transcript_fragment_from_path(
    transcript_path: &str,
    start_offset: i64,
) -> (String, Option<i64>) {
    if transcript_path.trim().is_empty() {
        return (String::new(), None);
    }
    let Ok(transcript_data) = std::fs::read(transcript_path) else {
        return (String::new(), None);
    };
    if transcript_data.is_empty() {
        return (String::new(), None);
    }

    let end_offset = transcript_position_from_bytes(&transcript_data);
    let start_offset = start_offset.max(0) as usize;
    (
        transcript_fragment_from_bytes(&transcript_data, start_offset, end_offset),
        Some(end_offset as i64),
    )
}

fn structured_message_count(transcript_data: &[u8]) -> Option<usize> {
    serde_json::from_slice::<Value>(transcript_data)
        .ok()?
        .get("messages")?
        .as_array()
        .map(Vec::len)
}

fn structured_transcript_fragment_from_offsets(
    transcript_data: &[u8],
    start_offset: usize,
    end_offset: usize,
) -> Option<String> {
    let mut value = serde_json::from_slice::<Value>(transcript_data).ok()?;
    let messages = value.get_mut("messages")?.as_array_mut()?;
    if start_offset >= messages.len() {
        return Some(String::new());
    }
    let bounded_end = end_offset.min(messages.len());
    let fragment_messages = messages[start_offset..bounded_end].to_vec();
    *messages = fragment_messages;
    serde_json::to_string(&value).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        transcript_fragment_from_bytes, transcript_position_from_bytes,
        read_transcript_fragment_from_path,
    };

    #[test]
    fn structured_json_fragment_uses_message_offsets() {
        let transcript =
            br#"{"messages":[{"type":"user","content":"hello"},{"type":"assistant","content":"done"}]}"#;

        let fragment = transcript_fragment_from_bytes(transcript, 0, 2);

        assert!(fragment.contains("\"assistant\""));
        assert!(fragment.contains("\"done\""));
    }

    #[test]
    fn structured_json_position_uses_message_count() {
        let transcript =
            br#"{"messages":[{"type":"user","content":"hello"},{"type":"assistant","content":"done"}]}"#;

        assert_eq!(transcript_position_from_bytes(transcript), 2);
    }

    #[test]
    fn read_fragment_from_path_uses_structured_offsets() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let transcript_path = temp_dir.path().join("transcript.json");
        std::fs::write(
            &transcript_path,
            r#"{"messages":[{"type":"user","content":"hello"},{"type":"assistant","content":"done"}]}"#,
        )
        .expect("write transcript");

        let (fragment, end_offset) =
            read_transcript_fragment_from_path(transcript_path.to_string_lossy().as_ref(), 0);

        assert_eq!(end_offset, Some(2));
        assert!(fragment.contains("\"done\""));
    }
}
