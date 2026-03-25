use std::convert::TryFrom;

use crate::host::checkpoints::lifecycle::{LifecycleEvent, LifecycleEventType};

use super::CanonicalSessionDescriptor;

/// Host-owned lifecycle event kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanonicalLifecycleEventKind {
    #[default]
    SessionStart,
    TurnStart,
    TurnEnd,
    Compaction,
    SessionEnd,
    SubagentStart,
    SubagentEnd,
    Unknown(i32),
}

impl CanonicalLifecycleEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::TurnStart => "turn_start",
            Self::TurnEnd => "turn_end",
            Self::Compaction => "compaction",
            Self::SessionEnd => "session_end",
            Self::SubagentStart => "subagent_start",
            Self::SubagentEnd => "subagent_end",
            Self::Unknown(_) => "unknown",
        }
    }
}

impl From<&LifecycleEventType> for CanonicalLifecycleEventKind {
    fn from(event_type: &LifecycleEventType) -> Self {
        match event_type {
            LifecycleEventType::SessionStart => Self::SessionStart,
            LifecycleEventType::TurnStart => Self::TurnStart,
            LifecycleEventType::TurnEnd => Self::TurnEnd,
            LifecycleEventType::Compaction => Self::Compaction,
            LifecycleEventType::SessionEnd => Self::SessionEnd,
            LifecycleEventType::SubagentStart => Self::SubagentStart,
            LifecycleEventType::SubagentEnd => Self::SubagentEnd,
            LifecycleEventType::Unknown(code) => Self::Unknown(*code),
        }
    }
}

/// Host-owned lifecycle event model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalLifecycleEvent {
    pub kind: CanonicalLifecycleEventKind,
    pub session: CanonicalSessionDescriptor,
    pub prompt: Option<String>,
    pub tool_use_id: Option<String>,
    pub subagent_id: Option<String>,
    pub model: Option<String>,
}

impl CanonicalLifecycleEvent {
    pub fn new(kind: CanonicalLifecycleEventKind, session: CanonicalSessionDescriptor) -> Self {
        Self {
            kind,
            session,
            prompt: None,
            tool_use_id: None,
            subagent_id: None,
            model: None,
        }
    }

    pub fn with_prompt(mut self, prompt: impl AsRef<str>) -> Self {
        let prompt = prompt.as_ref().trim();
        if !prompt.is_empty() {
            self.prompt = Some(prompt.to_string());
        }
        self
    }

    pub fn with_tool_use_id(mut self, tool_use_id: impl AsRef<str>) -> Self {
        let tool_use_id = tool_use_id.as_ref().trim();
        if !tool_use_id.is_empty() {
            self.tool_use_id = Some(tool_use_id.to_string());
        }
        self
    }

    pub fn with_subagent_id(mut self, subagent_id: impl AsRef<str>) -> Self {
        let subagent_id = subagent_id.as_ref().trim();
        if !subagent_id.is_empty() {
            self.subagent_id = Some(subagent_id.to_string());
        }
        self
    }

    pub fn with_model(mut self, model: impl AsRef<str>) -> Self {
        let model = model.as_ref().trim();
        if !model.is_empty() {
            self.model = Some(model.to_string());
        }
        self
    }
}

impl From<&LifecycleEvent> for CanonicalLifecycleEvent {
    fn from(event: &LifecycleEvent) -> Self {
        let session = CanonicalSessionDescriptor::try_from(event)
            .unwrap_or_else(|_| CanonicalSessionDescriptor::default());
        let kind = event
            .event_type
            .as_ref()
            .map(CanonicalLifecycleEventKind::from)
            .unwrap_or_default();
        let mut canonical = Self::new(kind, session);

        if !event.prompt.trim().is_empty() {
            canonical = canonical.with_prompt(&event.prompt);
        }
        if !event.tool_use_id.trim().is_empty() {
            canonical = canonical.with_tool_use_id(&event.tool_use_id);
        }
        if !event.subagent_id.trim().is_empty() {
            canonical = canonical.with_subagent_id(&event.subagent_id);
        }
        if !event.model.trim().is_empty() {
            canonical = canonical.with_model(&event.model);
        }

        canonical
    }
}
