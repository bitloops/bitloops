//! Cursor canonical transcript entry derivation.
//!
//! Cursor's canonical timeline is **chat-only by design**: we emit one
//! `USER/CHAT` or `ASSISTANT/CHAT` per detected message in source order, strip
//! `<user_query>` wrapper tags from user prompts, and ignore `tool_use` items
//! embedded in assistant messages. Cursor tool calls surface separately under
//! the dashboard's Tools tab (fed by the `interaction_tool_uses` table via the
//! `TranscriptToolEventDeriver` path), so omitting them here is intentional —
//! not a deferred feature. Do not add `TOOL_USE` / `TOOL_RESULT` emission to
//! this deriver without first confirming the UX decision has changed.

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
    let cleaned = strip_redacted_markers(&cleaned);
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

/// Strip Cursor's opaque `[REDACTED]` placeholders from a piece of text.
///
/// Cursor injects literal `[REDACTED]` tokens into assistant messages where
/// upstream content (e.g., thinking, system context, internal tool plumbing)
/// has been removed before persisting the transcript. These add no signal for
/// the dashboard reader, so we drop them:
///
/// * A line whose entire content is `[REDACTED]` produces an empty string and
///   gets filtered by the caller's `text.is_empty()` check.
/// * Trailing `\n\n[REDACTED]` after real content is removed cleanly.
/// * Mid-text occurrences are removed and any resulting run of 3+ newlines is
///   collapsed back to a single blank-line paragraph break.
fn strip_redacted_markers(text: &str) -> String {
    if !text.contains("[REDACTED]") {
        return text.to_string();
    }
    let stripped = text.replace("[REDACTED]", "");
    // Collapse runs of 3+ consecutive newlines down to 2 (one blank line),
    // which is what you'd get if the marker had simply not been there.
    let mut out = String::with_capacity(stripped.len());
    let mut newline_run = 0;
    for ch in stripped.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push(ch);
            }
        } else {
            newline_run = 0;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{derive_transcript_entries, strip_redacted_markers};
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

    #[test]
    fn strip_redacted_markers_is_noop_when_marker_absent() {
        let input = "no redactions here";
        assert_eq!(strip_redacted_markers(input), input);
    }

    #[test]
    fn strip_redacted_markers_removes_standalone_marker() {
        // A line whose entire text is just the marker collapses to empty,
        // which the caller filters out via `text.is_empty()`.
        assert_eq!(strip_redacted_markers("[REDACTED]"), "");
    }

    #[test]
    fn strip_redacted_markers_strips_trailing_marker_block() {
        // Pattern from the real Cursor JSONL: real content, blank line, marker.
        let input =
            "The README top-level heading is now `# cursor` (it was `# gem`).\n\n[REDACTED]";
        let out = strip_redacted_markers(input);
        let trimmed = out.trim();
        assert_eq!(
            trimmed,
            "The README top-level heading is now `# cursor` (it was `# gem`)."
        );
    }

    #[test]
    fn strip_redacted_markers_collapses_triple_newlines_left_by_mid_text_strip() {
        let input = "before\n\n[REDACTED]\n\nafter";
        // After replacing the marker we get "before\n\n\n\nafter"; collapse to "before\n\nafter".
        assert_eq!(strip_redacted_markers(input), "before\n\nafter");
    }

    #[test]
    fn assistant_chat_entry_drops_trailing_redacted_block() {
        // Mirrors a single line from a real Cursor JSONL session: real summary
        // text followed by a blank line and the [REDACTED] placeholder.
        let fragment = "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"The README top-level heading is now `# cursor` (it was `# gem`).\\n\\n[REDACTED]\"}]}}\n";
        let entries = derive_transcript_entries("sess-r", Some("turn-r"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.actor, TranscriptActor::Assistant);
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.source, TranscriptSource::Transcript);
        assert_eq!(
            entry.text,
            "The README top-level heading is now `# cursor` (it was `# gem`)."
        );
        assert!(!entry.text.contains("[REDACTED]"));
    }

    #[test]
    fn assistant_entry_whose_only_text_is_redacted_is_skipped() {
        // Some Cursor assistant lines carry only a `[REDACTED]` text item
        // alongside a tool_use. `content_to_text` already ignores tool_use, and
        // the remaining text is the marker alone — the entry must not be
        // emitted (it would otherwise render as an empty assistant bubble).
        let fragment = "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"[REDACTED]\"},{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{}}]}}\n";
        let entries = derive_transcript_entries("sess-r", Some("turn-r"), fragment)
            .expect("derive entries");
        assert!(
            entries.is_empty(),
            "redacted-only assistant entries should be filtered, got {entries:?}"
        );
    }
}
