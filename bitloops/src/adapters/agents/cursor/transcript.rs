//! Cursor canonical transcript entry derivation.
//!
//! Cursor has no agent-specific transcript parser yet. To still produce
//! canonical `TranscriptEntry` rows for Cursor sessions, we walk the
//! transcript line by line using the host-level role/content helpers, emit
//! one `USER/CHAT` or `ASSISTANT/CHAT` per detected message in source order,
//! and strip the `<user_query>` wrapper tags Cursor applies to user prompts.
//!
//! Tool traces are deferred until Cursor-specific fixtures exist.

use anyhow::Result;
use serde_json::Value;

use crate::adapters::agents::TranscriptEntryDeriver;
use crate::host::checkpoints::transcript::metadata::{
    content_to_text, transcript_line_content, transcript_line_role,
};
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_entry_id,
};

use super::agent::CursorAgent;

impl TranscriptEntryDeriver for CursorAgent {
    fn derive_transcript_entries(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        transcript: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        derive_transcript_entries(session_id, turn_id, transcript)
    }
}

/// Derive canonical chat entries for a Cursor transcript.
///
/// Walks the transcript line by line, classifies each line by role
/// (user vs assistant) using the host-level helpers, and emits a `USER/CHAT`
/// or `ASSISTANT/CHAT` per message in *source order*. Strips the
/// `<user_query>` wrapper Cursor applies to user prompts.
///
/// Falls back to the JSON-document shape (older fixtures and tests) when no
/// JSONL lines are recognised.
pub fn derive_transcript_entries(
    session_id: &str,
    turn_id: Option<&str>,
    transcript: &str,
) -> Result<Vec<TranscriptEntry>> {
    let scope = match turn_id {
        Some(id) => DerivationScope::Turn(id),
        None => DerivationScope::Session,
    };
    let mut entries: Vec<TranscriptEntry> = Vec::new();
    let mut order: i32 = 0;

    let mut pushed_any = false;
    for line in transcript.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if push_entry_from_value(session_id, &scope, &mut entries, &mut order, &value) {
            pushed_any = true;
        }
    }

    if !pushed_any
        && let Ok(value) = serde_json::from_str::<Value>(transcript)
        && let Some(messages) = value.get("messages").and_then(Value::as_array)
    {
        for message in messages {
            push_entry_from_value(session_id, &scope, &mut entries, &mut order, message);
        }
    }

    Ok(entries)
}

/// Returns `true` if it pushed an entry (so callers can detect whether any
/// JSONL line produced a recognised message).
fn push_entry_from_value(
    session_id: &str,
    scope: &DerivationScope<'_>,
    entries: &mut Vec<TranscriptEntry>,
    order: &mut i32,
    value: &Value,
) -> bool {
    let role = transcript_line_role(value);
    let Some(content) = transcript_line_content(value) else {
        return false;
    };
    let raw_text = content_to_text(content);
    let cleaned = strip_user_query_tags(&raw_text);
    let text = cleaned.trim();
    if text.is_empty() {
        return false;
    }

    let actor = if is_cursor_user_role(role) {
        TranscriptActor::User
    } else if is_cursor_assistant_role(role) {
        TranscriptActor::Assistant
    } else {
        return false;
    };

    entries.push(TranscriptEntry {
        entry_id: make_entry_id(session_id, scope, *order),
        session_id: session_id.to_string(),
        turn_id: scope.turn_id().map(str::to_string),
        order: *order,
        timestamp: None,
        actor,
        variant: TranscriptVariant::Chat,
        source: TranscriptSource::Transcript,
        text: text.to_string(),
        tool_use_id: None,
        tool_kind: None,
        is_error: false,
    });
    *order += 1;
    true
}

fn is_cursor_user_role(role: Option<&str>) -> bool {
    matches!(role, Some("user") | Some("human") | Some("user.message"))
}

fn is_cursor_assistant_role(role: Option<&str>) -> bool {
    matches!(
        role,
        Some("assistant") | Some("assistant.message") | Some("gemini")
    )
}

/// Strip `<user_query>` / `</user_query>` wrapper tags (case-insensitive) from a
/// Cursor user prompt. Cursor sometimes wraps the user prompt in these tags
/// internally; the dashboard should show the plain text.
fn strip_user_query_tags(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Hot path: only do work when one of the tag markers is present.
    let lower = text.to_ascii_lowercase();
    if !lower.contains("<user_query>") && !lower.contains("</user_query>") {
        return text.to_string();
    }
    // Walk the source, copying characters and skipping over either tag when we
    // hit it (case-insensitive). Order doesn't matter — both can appear.
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let rest_lower = &lower[i..];
        if rest_lower.starts_with("<user_query>") {
            i += "<user_query>".len();
        } else if rest_lower.starts_with("</user_query>") {
            i += "</user_query>".len();
        } else {
            // Copy one UTF-8 codepoint to avoid splitting characters.
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::derive_transcript_entries;
    use crate::host::interactions::transcript_entry::{
        TranscriptActor, TranscriptSource, TranscriptVariant,
    };

    #[test]
    fn claude_shape_user_message_produces_user_chat_entry() {
        let fragment = "{\"type\":\"user\",\"message\":{\"content\":\"hello cursor\"}}\n";
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        let user_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.actor == TranscriptActor::User)
            .collect();
        assert_eq!(user_entries.len(), 1);
        let entry = user_entries[0];
        assert_eq!(entry.actor, TranscriptActor::User);
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.text, "hello cursor");
        assert_eq!(entry.session_id, "sess-1");
        assert_eq!(entry.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(entry.order, 0);
        assert_eq!(entry.source, TranscriptSource::Transcript);
        assert!(entry.tool_use_id.is_none());
        assert!(entry.tool_kind.is_none());
        assert!(!entry.is_error);
        assert!(entry.timestamp.is_none());
    }

    #[test]
    fn claude_shape_assistant_summary_produces_assistant_chat_entry() {
        let fragment = concat!(
            "{\"type\":\"user\",\"message\":{\"content\":\"please summarise\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"all done\"}}\n",
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        let assistant_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.actor == TranscriptActor::Assistant)
            .collect();
        assert_eq!(assistant_entries.len(), 1);
        let entry = assistant_entries[0];
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.text, "all done");
        assert_eq!(entry.source, TranscriptSource::Transcript);
        assert!(entry.tool_use_id.is_none());
        assert!(entry.tool_kind.is_none());
        assert!(!entry.is_error);
        // The assistant chat row is emitted after all user prompts.
        let last = entries.last().expect("at least one entry");
        assert_eq!(last.actor, TranscriptActor::Assistant);
    }

    #[test]
    fn multiple_user_prompts_preserve_order() {
        let fragment = concat!(
            "{\"type\":\"user\",\"message\":{\"content\":\"first\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"ack\"}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":\"second\"}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":\"third\"}}\n",
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        let user_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.actor == TranscriptActor::User)
            .collect();
        assert_eq!(user_entries.len(), 3);
        assert_eq!(user_entries[0].text, "first");
        assert_eq!(user_entries[1].text, "second");
        assert_eq!(user_entries[2].text, "third");
        // Order indices are strictly increasing across the full entry list,
        // starting at 0, and user entries appear before the assistant entry.
        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.order, idx as i32);
        }
    }

    #[test]
    fn empty_transcript_returns_no_entries() {
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), "").expect("derive entries");
        assert!(entries.is_empty());
    }

    #[test]
    fn derived_entries_have_session_scope_when_turn_id_is_none() {
        let fragment = concat!(
            "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"hi\"}}\n",
        );
        let entries =
            derive_transcript_entries("sess-9", None, fragment).expect("derive entries");
        assert!(!entries.is_empty());
        for entry in &entries {
            assert!(entry.turn_id.is_none());
            assert!(
                entry.entry_id.contains(":session:"),
                "entry_id {:?} should include the session scope label",
                entry.entry_id
            );
        }
    }
}
