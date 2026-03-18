//! Host-owned canonical agent contract types.
//!
//! Adapter-specific structs under `engine/agent/<adapter>/...` are allowed to
//! keep target quirks. These types are the small, stable surface that host code
//! should use when it needs to reason about agents without binding to a
//! particular remote runtime.

use std::convert::TryFrom;

use anyhow::{Result, anyhow};

use super::{AgentAdapterRegistry, agent_display_name, canonical_agent_key};
use crate::engine::lifecycle::{LifecycleEvent, LifecycleEventType};

fn canonicalise_agent_key(raw: impl AsRef<str>) -> String {
    let collapsed = raw
        .as_ref()
        .trim()
        .to_ascii_lowercase()
        .split(|ch: char| ch.is_whitespace() || ch == '_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    canonical_agent_key(&collapsed)
}

/// Host-owned identity for an agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalAgentIdentity {
    pub agent_key: String,
    pub display_name: String,
}

impl CanonicalAgentIdentity {
    pub fn new(agent_key: impl AsRef<str>, display_name: impl AsRef<str>) -> Result<Self> {
        let agent_key = canonicalise_agent_key(agent_key);
        if agent_key.trim().is_empty() {
            return Err(anyhow!("agent key is required"));
        }

        let display_name = display_name.as_ref().trim();
        let display_name = if display_name.is_empty() {
            agent_display_name(&agent_key)
        } else {
            display_name.to_string()
        };
        Ok(Self {
            agent_key,
            display_name,
        })
    }

    pub fn from_agent_type(agent_type: impl AsRef<str>) -> Result<Self> {
        let agent_key = canonicalise_agent_key(agent_type);
        Self::new(&agent_key, agent_display_name(&agent_key))
    }
}

/// Host-owned session descriptor.
///
/// The descriptor is intentionally small: it carries the stable session ID and
/// an optional session reference path. Adapter-specific metadata stays outside
/// this contract.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalSessionDescriptor {
    pub session_id: String,
    pub session_ref: Option<String>,
}

impl CanonicalSessionDescriptor {
    pub fn new(session_id: impl AsRef<str>) -> Result<Self> {
        let session_id = session_id.as_ref().trim();
        if session_id.is_empty() {
            return Err(anyhow!("session_id is required"));
        }

        Ok(Self {
            session_id: session_id.to_string(),
            session_ref: None,
        })
    }

    pub fn with_session_ref(mut self, session_ref: impl AsRef<str>) -> Self {
        let session_ref = session_ref.as_ref().trim();
        if !session_ref.is_empty() {
            self.session_ref = Some(session_ref.to_string());
        }
        self
    }
}

impl TryFrom<&LifecycleEvent> for CanonicalSessionDescriptor {
    type Error = anyhow::Error;

    fn try_from(event: &LifecycleEvent) -> Result<Self> {
        Ok(Self::new(&event.session_id)?.with_session_ref(&event.session_ref))
    }
}

/// Host-owned behavioural flags.
///
/// These describe what the host can do; they are not adapter-specific feature
/// toggles and should stay generic across runtimes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCapabilityFlags {
    pub can_install_hooks: bool,
    pub can_resume_sessions: bool,
    pub can_read_transcripts: bool,
    pub can_write_transcripts: bool,
    pub can_report_token_usage: bool,
    pub can_observe_lifecycle_events: bool,
}

impl Default for HostCapabilityFlags {
    fn default() -> Self {
        Self::disabled()
    }
}

impl HostCapabilityFlags {
    pub const fn disabled() -> Self {
        Self {
            can_install_hooks: false,
            can_resume_sessions: false,
            can_read_transcripts: false,
            can_write_transcripts: false,
            can_report_token_usage: false,
            can_observe_lifecycle_events: false,
        }
    }

    pub const fn all_enabled() -> Self {
        Self {
            can_install_hooks: true,
            can_resume_sessions: true,
            can_read_transcripts: true,
            can_write_transcripts: true,
            can_report_token_usage: true,
            can_observe_lifecycle_events: true,
        }
    }
}

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

/// Host-owned request sent into an invocation boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalCorrelationMetadata {
    pub correlation_id: String,
    pub protocol_family: String,
    pub target_profile: String,
    pub runtime: String,
    pub resolution_path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalInvocationRequest {
    pub agent: CanonicalAgentIdentity,
    pub session: CanonicalSessionDescriptor,
    pub lifecycle_event: Option<CanonicalLifecycleEvent>,
    pub prompt: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub correlation: Option<CanonicalCorrelationMetadata>,
    pub capabilities: HostCapabilityFlags,
}

impl CanonicalInvocationRequest {
    pub fn new(agent: CanonicalAgentIdentity, session: CanonicalSessionDescriptor) -> Self {
        Self {
            agent,
            session,
            lifecycle_event: None,
            prompt: None,
            tool_name: None,
            tool_use_id: None,
            correlation: None,
            capabilities: HostCapabilityFlags::default(),
        }
    }

    pub fn with_lifecycle_event(mut self, lifecycle_event: CanonicalLifecycleEvent) -> Self {
        self.prompt = lifecycle_event.prompt.clone();
        self.tool_use_id = lifecycle_event.tool_use_id.clone();
        self.lifecycle_event = Some(lifecycle_event);
        self
    }

    pub fn with_prompt(mut self, prompt: impl AsRef<str>) -> Self {
        let prompt = prompt.as_ref().trim();
        if !prompt.is_empty() {
            self.prompt = Some(prompt.to_string());
        }
        self
    }

    pub fn with_tool_name(mut self, tool_name: impl AsRef<str>) -> Self {
        let tool_name = tool_name.as_ref().trim();
        if !tool_name.is_empty() {
            self.tool_name = Some(tool_name.to_string());
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

    pub fn with_correlation(mut self, correlation: CanonicalCorrelationMetadata) -> Self {
        self.correlation = Some(correlation);
        self
    }

    pub fn with_capabilities(mut self, capabilities: HostCapabilityFlags) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn for_lifecycle_event(
        agent_type: impl AsRef<str>,
        event: &LifecycleEvent,
    ) -> Result<Self> {
        let raw_agent_type = agent_type.as_ref().trim().to_string();
        let agent = CanonicalAgentIdentity::from_agent_type(&raw_agent_type)?;
        let session = CanonicalSessionDescriptor::try_from(event)?;
        let lifecycle_event = CanonicalLifecycleEvent::from(event);

        let mut request = Self::new(agent, session).with_lifecycle_event(lifecycle_event);
        if !event.prompt.trim().is_empty() {
            request = request.with_prompt(&event.prompt);
        }
        if !event.tool_use_id.trim().is_empty() {
            request = request.with_tool_use_id(&event.tool_use_id);
        }

        if let Ok(resolved) =
            AgentAdapterRegistry::builtin().resolve_with_trace(&request.agent.agent_key, None)
        {
            let descriptor = resolved.registration.descriptor();
            if let Ok(identity) =
                CanonicalAgentIdentity::new(descriptor.agent_type, descriptor.display_name)
            {
                request.agent = identity;
            }
            request = request.with_correlation(CanonicalCorrelationMetadata {
                correlation_id: resolved.trace.correlation_id,
                protocol_family: descriptor.protocol_family.id.to_string(),
                target_profile: descriptor.target_profile.id.to_string(),
                runtime: resolved.trace.runtime,
                resolution_path: resolved.trace.resolution_path,
            });
        }

        Ok(request)
    }
}

/// Host-owned invocation response model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalInvocationResponse {
    pub session: CanonicalSessionDescriptor,
    pub lifecycle_event: Option<CanonicalLifecycleEvent>,
    pub output: Option<String>,
    pub modified_files: Vec<String>,
}

impl CanonicalInvocationResponse {
    pub fn new(session: CanonicalSessionDescriptor) -> Self {
        Self {
            session,
            lifecycle_event: None,
            output: None,
            modified_files: Vec::new(),
        }
    }

    pub fn with_output(mut self, output: impl AsRef<str>) -> Self {
        let output = output.as_ref().trim();
        if !output.is_empty() {
            self.output = Some(output.to_string());
        }
        self
    }

    pub fn with_lifecycle_event(mut self, lifecycle_event: CanonicalLifecycleEvent) -> Self {
        self.lifecycle_event = Some(lifecycle_event);
        self
    }

    pub fn with_modified_files(mut self, modified_files: Vec<String>) -> Self {
        self.modified_files = modified_files;
        self
    }
}

/// Host-owned failure model for invocation boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalInvocationFailureKind {
    InvalidRequest,
    Unsupported,
    Transient,
    Fatal,
}

impl CanonicalInvocationFailureKind {
    fn default_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalInvocationFailure {
    pub session: Option<CanonicalSessionDescriptor>,
    pub lifecycle_event: Option<CanonicalLifecycleEvent>,
    pub kind: CanonicalInvocationFailureKind,
    pub message: String,
    pub retryable: bool,
}

impl Default for CanonicalInvocationFailure {
    fn default() -> Self {
        Self {
            session: None,
            lifecycle_event: None,
            kind: CanonicalInvocationFailureKind::InvalidRequest,
            message: String::new(),
            retryable: false,
        }
    }
}

impl CanonicalInvocationFailure {
    pub fn new(kind: CanonicalInvocationFailureKind, message: impl AsRef<str>) -> Self {
        Self {
            session: None,
            lifecycle_event: None,
            kind,
            message: message.as_ref().trim().to_string(),
            retryable: kind.default_retryable(),
        }
    }

    pub fn with_session(mut self, session: CanonicalSessionDescriptor) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_lifecycle_event(mut self, lifecycle_event: CanonicalLifecycleEvent) -> Self {
        self.lifecycle_event = Some(lifecycle_event);
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_identity_normalises_agent_keys_and_display_names() {
        let identity = CanonicalAgentIdentity::new(" Claude Code ", " ").expect("identity");
        assert_eq!(identity.agent_key, "claude-code");
        assert_eq!(identity.display_name, "Claude Code");

        let derived = CanonicalAgentIdentity::from_agent_type("gemini").expect("identity");
        assert_eq!(derived.agent_key, "gemini");
        assert_eq!(derived.display_name, "Gemini");
    }

    #[test]
    fn session_descriptor_requires_a_session_id() {
        assert!(CanonicalSessionDescriptor::new(" ").is_err());

        let session = CanonicalSessionDescriptor::new("  session-123  ")
            .expect("session")
            .with_session_ref(" /tmp/session.jsonl ");
        assert_eq!(session.session_id, "session-123");
        assert_eq!(session.session_ref.as_deref(), Some("/tmp/session.jsonl"));
    }

    #[test]
    fn host_capability_flags_default_to_disabled() {
        let flags = HostCapabilityFlags::default();
        assert!(!flags.can_install_hooks);
        assert!(!flags.can_resume_sessions);
        assert!(!flags.can_read_transcripts);
        assert!(!flags.can_write_transcripts);
        assert!(!flags.can_report_token_usage);
        assert!(!flags.can_observe_lifecycle_events);

        let enabled = HostCapabilityFlags::all_enabled();
        assert!(enabled.can_install_hooks);
        assert!(enabled.can_resume_sessions);
        assert!(enabled.can_read_transcripts);
        assert!(enabled.can_write_transcripts);
        assert!(enabled.can_report_token_usage);
        assert!(enabled.can_observe_lifecycle_events);
    }

    #[test]
    fn lifecycle_event_kind_maps_unknown_and_known_variants() {
        assert_eq!(
            CanonicalLifecycleEventKind::from(&LifecycleEventType::SessionStart),
            CanonicalLifecycleEventKind::SessionStart
        );
        assert_eq!(
            CanonicalLifecycleEventKind::from(&LifecycleEventType::TurnEnd),
            CanonicalLifecycleEventKind::TurnEnd
        );
        assert_eq!(
            CanonicalLifecycleEventKind::from(&LifecycleEventType::Unknown(42)),
            CanonicalLifecycleEventKind::Unknown(42)
        );
        assert_eq!(CanonicalLifecycleEventKind::Unknown(42).as_str(), "unknown");
    }

    #[test]
    fn lifecycle_event_conversion_trims_host_owned_values() {
        let event = LifecycleEvent {
            event_type: Some(LifecycleEventType::TurnStart),
            session_id: " session-7 ".to_string(),
            session_ref: " /tmp/session-7.jsonl ".to_string(),
            prompt: "  hello world  ".to_string(),
            tool_use_id: "  tool-1  ".to_string(),
            subagent_id: "  subagent-9  ".to_string(),
            model: "  gemini-2.5  ".to_string(),
        };

        let canonical = CanonicalLifecycleEvent::from(&event);
        assert_eq!(canonical.kind, CanonicalLifecycleEventKind::TurnStart);
        assert_eq!(canonical.session.session_id, "session-7");
        assert_eq!(
            canonical.session.session_ref.as_deref(),
            Some("/tmp/session-7.jsonl")
        );
        assert_eq!(canonical.prompt.as_deref(), Some("hello world"));
        assert_eq!(canonical.tool_use_id.as_deref(), Some("tool-1"));
        assert_eq!(canonical.subagent_id.as_deref(), Some("subagent-9"));
        assert_eq!(canonical.model.as_deref(), Some("gemini-2.5"));
    }

    #[test]
    fn invocation_request_builds_from_lifecycle_event() {
        let event = LifecycleEvent {
            event_type: Some(LifecycleEventType::SessionStart),
            session_id: "session-42".to_string(),
            session_ref: "/tmp/session-42.jsonl".to_string(),
            prompt: "  ping  ".to_string(),
            tool_use_id: "  tool-use-42  ".to_string(),
            ..LifecycleEvent::default()
        };

        let request = CanonicalInvocationRequest::for_lifecycle_event("Claude Code", &event)
            .expect("request");
        assert_eq!(request.agent.agent_key, "claude-code");
        assert_eq!(request.agent.display_name, "Claude Code");
        assert_eq!(request.session.session_id, "session-42");
        assert_eq!(
            request.session.session_ref.as_deref(),
            Some("/tmp/session-42.jsonl")
        );
        assert_eq!(request.prompt.as_deref(), Some("ping"));
        assert_eq!(request.tool_use_id.as_deref(), Some("tool-use-42"));
        assert!(request.lifecycle_event.is_some());
        assert!(request.correlation.is_some());
        let correlation = request.correlation.as_ref().expect("correlation");
        assert_eq!(correlation.protocol_family, "jsonl-cli");
        assert_eq!(correlation.target_profile, "claude-code");
        assert!(!correlation.correlation_id.is_empty());
        assert!(request.capabilities == HostCapabilityFlags::default());
    }

    #[test]
    fn invocation_response_and_failure_preserve_session_context() {
        let session = CanonicalSessionDescriptor::new("session-99")
            .expect("session")
            .with_session_ref("/tmp/session-99.jsonl");
        let lifecycle_event =
            CanonicalLifecycleEvent::new(CanonicalLifecycleEventKind::SessionEnd, session.clone());

        let response = CanonicalInvocationResponse::new(session.clone())
            .with_lifecycle_event(lifecycle_event.clone())
            .with_output("  done  ")
            .with_modified_files(vec!["src/main.rs".to_string()]);
        assert_eq!(response.session, session);
        assert_eq!(response.output.as_deref(), Some("done"));
        assert_eq!(response.modified_files, vec!["src/main.rs".to_string()]);
        assert_eq!(response.lifecycle_event.as_ref(), Some(&lifecycle_event));

        let failure = CanonicalInvocationFailure::new(
            CanonicalInvocationFailureKind::Transient,
            "  temporary failure  ",
        )
        .with_session(session.clone())
        .with_lifecycle_event(lifecycle_event);
        assert_eq!(failure.session.as_ref(), Some(&session));
        assert_eq!(failure.message, "temporary failure");
        assert!(failure.retryable);
    }
}
