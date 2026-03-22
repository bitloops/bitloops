//! Commit trailer parsing/formatting
pub const STRATEGY_TRAILER_KEY: &str = "Bitloops-Strategy";
pub const METADATA_TRAILER_KEY: &str = "Bitloops-Metadata";
pub const METADATA_TASK_TRAILER_KEY: &str = "Bitloops-Metadata-Task";
pub const BASE_COMMIT_TRAILER_KEY: &str = "Base-Commit";
pub const SESSION_TRAILER_KEY: &str = "Bitloops-Session";
pub const CONDENSATION_TRAILER_KEY: &str = "Bitloops-Condensation";
pub const SOURCE_REF_TRAILER_KEY: &str = "Bitloops-Source-Ref";
pub const CHECKPOINT_TRAILER_KEY: &str = "Bitloops-Checkpoint";
pub const EPHEMERAL_BRANCH_TRAILER_KEY: &str = "Ephemeral-branch";
pub const AGENT_TRAILER_KEY: &str = "Bitloops-Agent";
pub const CHECKPOINT_ID_PATTERN: &str = "[0-9a-f]{12}";
pub const SHORT_ID_LENGTH: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckpointId(String);

impl CheckpointId {
    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn try_new(value: &str) -> Option<Self> {
        if is_valid_checkpoint_id(value) {
            return Some(Self(value.to_string()));
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub fn is_valid_checkpoint_id(value: &str) -> bool {
    if value.len() != SHORT_ID_LENGTH {
        return false;
    }
    value
        .as_bytes()
        .iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub fn parse_metadata(commit_message: &str) -> (String, bool) {
    if let Some(value) = find_first_line_value(commit_message, METADATA_TRAILER_KEY) {
        return (value.trim().to_string(), true);
    }
    (String::new(), false)
}

pub fn parse_session(commit_message: &str) -> (String, bool) {
    if let Some(value) = find_first_line_value(commit_message, SESSION_TRAILER_KEY) {
        return (value.trim().to_string(), true);
    }
    (String::new(), false)
}

pub fn format_checkpoint(message: &str, checkpoint_id: &CheckpointId) -> String {
    format!(
        "{message}\n\n{CHECKPOINT_TRAILER_KEY}: {}\n",
        checkpoint_id.as_str()
    )
}

pub fn parse_checkpoint(commit_message: &str) -> (CheckpointId, bool) {
    let mut search_start = 0usize;
    while let Some(key_pos) =
        find_key_at_or_after(commit_message, CHECKPOINT_TRAILER_KEY, search_start)
    {
        let value_start =
            skip_whitespace_after_colon(commit_message, key_pos, CHECKPOINT_TRAILER_KEY);
        if let Some(value) = fixed_lower_hex_prefix(commit_message, value_start, SHORT_ID_LENGTH) {
            let next_index = value_start + SHORT_ID_LENGTH;
            if (next_index == commit_message.len()
                || commit_message[next_index..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace))
                && let Some(id) = CheckpointId::try_new(value)
            {
                return (id, true);
            }
        }
        search_start = key_pos + 1;
    }
    (CheckpointId::empty(), false)
}

fn find_key_at_or_after(message: &str, key: &str, start: usize) -> Option<usize> {
    let needle = format!("{key}:");
    message
        .get(start..)?
        .find(&needle)
        .map(|relative| start + relative)
}

fn skip_whitespace_after_colon(message: &str, key_pos: usize, key: &str) -> usize {
    let mut index = key_pos + key.len() + 1;
    while index < message.len() {
        let ch = message[index..].chars().next().unwrap_or('\0');
        if ch.is_whitespace() {
            index += ch.len_utf8();
            continue;
        }
        break;
    }
    index
}

fn find_first_line_value<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let mut search_start = 0usize;
    while let Some(key_pos) = find_key_at_or_after(message, key, search_start) {
        let value_start = skip_whitespace_after_colon(message, key_pos, key);
        if value_start < message.len() {
            let value_end = value_start
                + message[value_start..]
                    .find('\n')
                    .unwrap_or(message.len() - value_start);
            if value_end > value_start {
                return message.get(value_start..value_end);
            }
        }
        search_start = key_pos + 1;
    }
    None
}

fn fixed_lower_hex_prefix(message: &str, start: usize, len: usize) -> Option<&str> {
    let end = start.checked_add(len)?;
    if end > message.len() {
        return None;
    }
    let candidate = message.get(start..end)?;
    if candidate
        .as_bytes()
        .iter()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Some(candidate);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metadata_matches_go_cases() {
        let tests = [
            (
                "standard commit message",
                "Update logic\n\nBitloops-Metadata: .bitloops/metadata/2025-01-28-abc123\n",
                ".bitloops/metadata/2025-01-28-abc123",
                true,
            ),
            ("no trailer", "Simple commit message", "", false),
            (
                "trailer with extra spaces",
                "Message\n\nBitloops-Metadata:   .bitloops/metadata/xyz   \n",
                ".bitloops/metadata/xyz",
                true,
            ),
        ];

        for (name, message, want_dir, want_found) in tests {
            let (got_dir, got_found) = parse_metadata(message);
            assert_eq!(
                got_found, want_found,
                "{name}: parse_metadata found mismatch"
            );
            assert_eq!(got_dir, want_dir, "{name}: parse_metadata dir mismatch");
        }
    }

    #[test]
    fn parse_session_matches_go_cases() {
        let tests = [
            (
                "single session trailer",
                "Update logic\n\nBitloops-Session: 2025-12-10-abc123def\n",
                "2025-12-10-abc123def",
                true,
            ),
            ("no trailer", "Simple commit message", "", false),
            (
                "trailer with extra spaces",
                "Message\n\nBitloops-Session:   2025-12-10-xyz789   \n",
                "2025-12-10-xyz789",
                true,
            ),
            (
                "multiple trailers returns first",
                "Merge\n\nBitloops-Session: session-1\nBitloops-Session: session-2\n",
                "session-1",
                true,
            ),
        ];

        for (name, message, want_id, want_found) in tests {
            let (got_id, got_found) = parse_session(message);
            assert_eq!(
                got_found, want_found,
                "{name}: parse_session found mismatch"
            );
            assert_eq!(got_id, want_id, "{name}: parse_session id mismatch");
        }
    }

    #[test]
    fn parse_checkpoint_matches_go_cases() {
        let tests = [
            (
                "valid checkpoint trailer",
                "Add feature\n\nBitloops-Checkpoint: a1b2c3d4e5f6\n",
                "a1b2c3d4e5f6",
                true,
            ),
            ("no trailer", "Simple commit message", "", false),
            (
                "trailer with extra spaces",
                "Message\n\nBitloops-Checkpoint:   a1b2c3d4e5f6   \n",
                "a1b2c3d4e5f6",
                true,
            ),
            (
                "too short checkpoint ID",
                "Message\n\nBitloops-Checkpoint: abc123\n",
                "",
                false,
            ),
            (
                "too long checkpoint ID",
                "Message\n\nBitloops-Checkpoint: a1b2c3d4e5f6789\n",
                "",
                false,
            ),
            (
                "invalid characters in checkpoint ID",
                "Message\n\nBitloops-Checkpoint: a1b2c3d4e5gg\n",
                "",
                false,
            ),
            (
                "uppercase hex rejected",
                "Message\n\nBitloops-Checkpoint: A1B2C3D4E5F6\n",
                "",
                false,
            ),
        ];

        for (name, message, want_id, want_found) in tests {
            let (got_id, got_found) = parse_checkpoint(message);
            assert_eq!(
                got_found, want_found,
                "{name}: parse_checkpoint found mismatch"
            );
            assert_eq!(
                got_id.as_str(),
                want_id,
                "{name}: parse_checkpoint id mismatch"
            );
        }
    }
}
