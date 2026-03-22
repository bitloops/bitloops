use std::path::Path;
use std::{fs::File, io::Write};

use anyhow::{Context, Result};

use super::types::Line;

pub fn write_transcript(path: &Path, transcript: &[Line]) -> Result<()> {
    let mut file = File::create(path)
        .with_context(|| format!("failed to create file: {}", path.to_string_lossy()))?;

    for line in transcript {
        let data = serde_json::to_vec(line).context("failed to marshal line")?;
        file.write_all(&data).context("failed to write line")?;
        file.write_all(b"\n").context("failed to write newline")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::NamedTempFile;

    use super::write_transcript;
    use crate::host::checkpoints::transcript::parse::parse_from_bytes;
    use crate::host::checkpoints::transcript::types::Line;

    #[test]
    fn test_write_transcript_jsonl_roundtrip() {
        let tmp = NamedTempFile::new().expect("failed to create temp file");
        let transcript = vec![
            Line {
                r#type: "user".to_string(),
                uuid: "u1".to_string(),
                message: json!({"content":"Hello"}),
            },
            Line {
                r#type: "assistant".to_string(),
                uuid: "a1".to_string(),
                message: json!({"content":[{"type":"text","text":"Hi"}]}),
            },
        ];

        write_transcript(tmp.path(), &transcript).expect("write_transcript should succeed");

        let bytes = fs::read(tmp.path()).expect("failed to read transcript file");
        assert!(
            String::from_utf8(bytes.clone())
                .expect("transcript must be valid utf-8")
                .ends_with('\n'),
            "expected JSONL to end with newline"
        );

        let parsed = parse_from_bytes(&bytes).expect("failed to parse written transcript");
        assert_eq!(parsed, transcript);
    }

    #[test]
    fn test_write_transcript_empty() {
        let tmp = NamedTempFile::new().expect("failed to create temp file");
        write_transcript(tmp.path(), &[]).expect("write_transcript should succeed for empty input");
        let bytes = fs::read(tmp.path()).expect("failed to read transcript file");
        assert!(bytes.is_empty(), "expected empty file for empty transcript");
    }
}
