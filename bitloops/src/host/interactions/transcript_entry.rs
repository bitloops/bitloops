//! Canonical transcript entry domain model.
//!
//! `TranscriptEntry` is the display-ready, agent-agnostic shape that the dashboard
//! consumes. Each agent's `TranscriptEntryDeriver` impl converts agent-specific
//! transcript content into a stream of these entries.
//!
//! See `docs/transcript-normalization` (or the Confluence draft) for the contract.

use serde::{Deserialize, Serialize};

/// Who produced this entry, from the reader's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptActor {
    User,
    Assistant,
    System,
}

impl TranscriptActor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }
}

/// The semantic kind of content this entry carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptVariant {
    /// Normal user or assistant message text.
    Chat,
    /// Assistant reasoning / thinking block.
    Thinking,
    /// A tool invocation row.
    ToolUse,
    /// A tool result row.
    ToolResult,
}

impl TranscriptVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Thinking => "thinking",
            Self::ToolUse => "tool_use",
            Self::ToolResult => "tool_result",
        }
    }
}

/// Where the entry's data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptSource {
    /// Derived from real transcript content.
    Transcript,
    /// Synthesised from `turn.prompt` when no transcript slice exists for a turn.
    PromptFallback,
}

impl TranscriptSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::PromptFallback => "prompt_fallback",
        }
    }
}

/// A single, display-ready transcript row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub entry_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub actor: TranscriptActor,
    pub variant: TranscriptVariant,
    pub source: TranscriptSource,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

/// Scope label used when generating deterministic ids.
///
/// `Turn(id)` means we're deriving entries for a specific turn slice. `Session`
/// means we're deriving over the whole session transcript before segmentation.
pub enum DerivationScope<'a> {
    Turn(&'a str),
    Session,
}

impl<'a> DerivationScope<'a> {
    pub fn label(&self) -> &str {
        match self {
            Self::Turn(id) => id,
            Self::Session => "session",
        }
    }

    pub fn turn_id(&self) -> Option<&str> {
        match self {
            Self::Turn(id) => Some(*id),
            Self::Session => None,
        }
    }
}

/// Deterministic id for a transcript entry within a (session, scope, order) tuple.
///
/// Stable across re-reads of the same transcript so the frontend can cache.
pub fn make_entry_id(session_id: &str, scope: &DerivationScope<'_>, order: i32) -> String {
    format!("entry:{}:{}:{:04}", session_id, scope.label(), order)
}

/// Deterministic `tool_use_id` when the source transcript does not provide one.
///
/// Format: `derived:<session_id>:<scope>:<tool_call_index>`. The `derived:` prefix
/// makes generated ids visually distinct from source-supplied ids during debugging.
pub fn make_derived_tool_use_id(
    session_id: &str,
    scope: &DerivationScope<'_>,
    tool_call_index: i32,
) -> String {
    format!(
        "derived:{}:{}:{}",
        session_id,
        scope.label(),
        tool_call_index
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transcript_entry_serde_round_trip() {
        let entry = TranscriptEntry {
            entry_id: "entry:sess-1:turn-1:0001".to_string(),
            session_id: "sess-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            order: 1,
            timestamp: Some("2026-04-04T10:01:00Z".to_string()),
            actor: TranscriptActor::Assistant,
            variant: TranscriptVariant::Chat,
            source: TranscriptSource::Transcript,
            text: "hello".to_string(),
            tool_use_id: None,
            tool_kind: None,
            is_error: false,
        };

        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["actor"], json!("assistant"));
        assert_eq!(json["variant"], json!("chat"));
        assert_eq!(json["source"], json!("transcript"));

        let parsed: TranscriptEntry = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn tool_use_entry_serde_round_trip() {
        let entry = TranscriptEntry {
            entry_id: "entry:sess-1:turn-1:0002".to_string(),
            session_id: "sess-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            order: 2,
            timestamp: None,
            actor: TranscriptActor::System,
            variant: TranscriptVariant::ToolUse,
            source: TranscriptSource::Transcript,
            text: "Tool: bash".to_string(),
            tool_use_id: Some("call_bash_123".to_string()),
            tool_kind: Some("bash".to_string()),
            is_error: false,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: TranscriptEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.variant, TranscriptVariant::ToolUse);
    }

    #[test]
    fn omits_optional_fields_when_none() {
        let entry = TranscriptEntry {
            entry_id: "entry:sess-1:session:0000".to_string(),
            session_id: "sess-1".to_string(),
            turn_id: None,
            order: 0,
            timestamp: None,
            actor: TranscriptActor::User,
            variant: TranscriptVariant::Chat,
            source: TranscriptSource::PromptFallback,
            text: "hi".to_string(),
            tool_use_id: None,
            tool_kind: None,
            is_error: false,
        };

        let json = serde_json::to_value(&entry).unwrap();
        assert!(json.get("turn_id").is_none());
        assert!(json.get("timestamp").is_none());
        assert!(json.get("tool_use_id").is_none());
        assert!(json.get("tool_kind").is_none());
        assert_eq!(json["source"], json!("prompt_fallback"));
    }

    #[test]
    fn entry_id_is_deterministic_per_scope() {
        let scope_turn = DerivationScope::Turn("turn-7");
        let scope_session = DerivationScope::Session;

        assert_eq!(
            make_entry_id("sess-x", &scope_turn, 3),
            "entry:sess-x:turn-7:0003"
        );
        assert_eq!(
            make_entry_id("sess-x", &scope_session, 12),
            "entry:sess-x:session:0012"
        );

        // Same inputs → same id.
        let a = make_entry_id("sess-x", &scope_turn, 3);
        let b = make_entry_id("sess-x", &scope_turn, 3);
        assert_eq!(a, b);
    }

    #[test]
    fn derived_tool_use_id_has_visible_prefix() {
        let id = make_derived_tool_use_id("sess-x", &DerivationScope::Turn("turn-2"), 1);
        assert_eq!(id, "derived:sess-x:turn-2:1");
        assert!(id.starts_with("derived:"));
    }

    #[test]
    fn actor_variant_source_str_round_trip() {
        for actor in [
            TranscriptActor::User,
            TranscriptActor::Assistant,
            TranscriptActor::System,
        ] {
            let s = actor.as_str();
            let json = serde_json::to_value(actor).unwrap();
            assert_eq!(json, json!(s));
        }

        for variant in [
            TranscriptVariant::Chat,
            TranscriptVariant::Thinking,
            TranscriptVariant::ToolUse,
            TranscriptVariant::ToolResult,
        ] {
            let s = variant.as_str();
            let json = serde_json::to_value(variant).unwrap();
            assert_eq!(json, json!(s));
        }

        for source in [TranscriptSource::Transcript, TranscriptSource::PromptFallback] {
            let s = source.as_str();
            let json = serde_json::to_value(source).unwrap();
            assert_eq!(json, json!(s));
        }
    }
}
