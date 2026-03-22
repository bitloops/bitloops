use std::convert::TryFrom;

use anyhow::Result;

use crate::adapters::agents::AgentAdapterRegistry;
use crate::host::checkpoints::lifecycle::LifecycleEvent;

use super::{
    CanonicalAgentIdentity, CanonicalContractCompatibility, CanonicalLifecycleEvent,
    CanonicalProgressUpdate, CanonicalResultFragment, CanonicalResumableSession,
    CanonicalSessionDescriptor, CanonicalStreamEvent, HostCapabilityFlags,
};

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
    pub compatibility: CanonicalContractCompatibility,
    pub prompt: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub progress: Option<CanonicalProgressUpdate>,
    pub resumable_session: Option<CanonicalResumableSession>,
    pub correlation: Option<CanonicalCorrelationMetadata>,
    pub capabilities: HostCapabilityFlags,
}

impl CanonicalInvocationRequest {
    pub fn new(agent: CanonicalAgentIdentity, session: CanonicalSessionDescriptor) -> Self {
        Self {
            agent,
            session,
            lifecycle_event: None,
            compatibility: CanonicalContractCompatibility::default(),
            prompt: None,
            tool_name: None,
            tool_use_id: None,
            progress: None,
            resumable_session: None,
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

    pub fn with_progress(mut self, progress: CanonicalProgressUpdate) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_resumable_session(mut self, resumable_session: CanonicalResumableSession) -> Self {
        self.resumable_session = Some(resumable_session);
        self
    }

    pub fn with_compatibility(mut self, compatibility: CanonicalContractCompatibility) -> Self {
        self.compatibility = compatibility;
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
    pub compatibility: CanonicalContractCompatibility,
    pub output: Option<String>,
    pub stream_events: Vec<CanonicalStreamEvent>,
    pub result_fragment: Option<CanonicalResultFragment>,
    pub progress: Option<CanonicalProgressUpdate>,
    pub resumable_session: Option<CanonicalResumableSession>,
    pub modified_files: Vec<String>,
}

impl CanonicalInvocationResponse {
    pub fn new(session: CanonicalSessionDescriptor) -> Self {
        Self {
            session,
            lifecycle_event: None,
            compatibility: CanonicalContractCompatibility::default(),
            output: None,
            stream_events: Vec::new(),
            result_fragment: None,
            progress: None,
            resumable_session: None,
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

    pub fn with_compatibility(mut self, compatibility: CanonicalContractCompatibility) -> Self {
        self.compatibility = compatibility;
        self
    }

    pub fn with_modified_files(mut self, modified_files: Vec<String>) -> Self {
        self.modified_files = modified_files;
        self
    }

    pub fn with_stream_event(mut self, event: CanonicalStreamEvent) -> Self {
        self.stream_events.push(event);
        self
    }

    pub fn with_stream_events(mut self, events: Vec<CanonicalStreamEvent>) -> Self {
        self.stream_events = events;
        self
    }

    pub fn with_result_fragment(mut self, result_fragment: CanonicalResultFragment) -> Self {
        self.result_fragment = Some(result_fragment);
        self
    }

    pub fn with_progress(mut self, progress: CanonicalProgressUpdate) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_resumable_session(mut self, resumable_session: CanonicalResumableSession) -> Self {
        self.resumable_session = Some(resumable_session);
        self
    }
}

/// Host-owned failure model for invocation boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanonicalInvocationFailureKind {
    #[default]
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalInvocationFailure {
    pub session: Option<CanonicalSessionDescriptor>,
    pub lifecycle_event: Option<CanonicalLifecycleEvent>,
    pub compatibility: CanonicalContractCompatibility,
    pub stream_events: Vec<CanonicalStreamEvent>,
    pub progress: Option<CanonicalProgressUpdate>,
    pub resumable_session: Option<CanonicalResumableSession>,
    pub kind: CanonicalInvocationFailureKind,
    pub message: String,
    pub retryable: bool,
}

impl CanonicalInvocationFailure {
    pub fn new(kind: CanonicalInvocationFailureKind, message: impl AsRef<str>) -> Self {
        Self {
            session: None,
            lifecycle_event: None,
            compatibility: CanonicalContractCompatibility::default(),
            stream_events: Vec::new(),
            progress: None,
            resumable_session: None,
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

    pub fn with_compatibility(mut self, compatibility: CanonicalContractCompatibility) -> Self {
        self.compatibility = compatibility;
        self
    }

    pub fn with_stream_event(mut self, event: CanonicalStreamEvent) -> Self {
        self.stream_events.push(event);
        self
    }

    pub fn with_progress(mut self, progress: CanonicalProgressUpdate) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_resumable_session(mut self, resumable_session: CanonicalResumableSession) -> Self {
        self.resumable_session = Some(resumable_session);
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}
