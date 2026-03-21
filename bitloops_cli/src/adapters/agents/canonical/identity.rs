use std::convert::TryFrom;

use anyhow::{Result, anyhow};

use crate::adapters::agents::{agent_display_name, canonical_agent_key};
use crate::engine::lifecycle::LifecycleEvent;

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
