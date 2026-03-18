//! Host-owned policy, provenance, and audit semantics for canonical agent flows.
//!
//! The module stays runtime-neutral so it can represent local and future remote
//! execution paths without binding policy decisions to a specific adapter.

use anyhow::{Result, bail};
use std::time::SystemTime;

use super::canonical::{
    CanonicalInvocationFailure, CanonicalInvocationRequest, CanonicalInvocationResponse,
    CanonicalSessionDescriptor,
};

fn trim_required(label: &str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref().trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value.to_string())
}

fn trim_optional(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref().trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyExplanation {
    pub code: String,
    pub reason: String,
}

impl PolicyExplanation {
    pub fn new(code: impl AsRef<str>, reason: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            code: trim_required("policy code", code)?,
            reason: trim_required("policy reason", reason)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRestriction {
    pub scope: String,
    pub description: String,
}

impl PolicyRestriction {
    pub fn new(scope: impl AsRef<str>, description: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            scope: trim_required("policy restriction scope", scope)?,
            description: trim_required("policy restriction description", description)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRedaction {
    pub field: String,
    pub replacement: String,
}

impl PolicyRedaction {
    pub fn new(field: impl AsRef<str>, replacement: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            field: trim_required("policy redaction field", field)?,
            replacement: replacement.as_ref().trim().to_string(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PolicyDecision {
    #[default]
    Allow,
    Deny {
        explanation: PolicyExplanation,
    },
    Restricted {
        explanation: PolicyExplanation,
        restrictions: Vec<PolicyRestriction>,
    },
    Redacted {
        explanation: PolicyExplanation,
        redactions: Vec<PolicyRedaction>,
    },
}

impl PolicyDecision {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn deny(code: impl AsRef<str>, reason: impl AsRef<str>) -> Result<Self> {
        Ok(Self::Deny {
            explanation: PolicyExplanation::new(code, reason)?,
        })
    }

    pub fn restricted(
        code: impl AsRef<str>,
        reason: impl AsRef<str>,
        restrictions: Vec<PolicyRestriction>,
    ) -> Result<Self> {
        Ok(Self::Restricted {
            explanation: PolicyExplanation::new(code, reason)?,
            restrictions,
        })
    }

    pub fn redacted(
        code: impl AsRef<str>,
        reason: impl AsRef<str>,
        redactions: Vec<PolicyRedaction>,
    ) -> Result<Self> {
        Ok(Self::Redacted {
            explanation: PolicyExplanation::new(code, reason)?,
            redactions,
        })
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    pub fn is_restricted(&self) -> bool {
        matches!(self, Self::Restricted { .. })
    }

    pub fn is_redacted(&self) -> bool {
        matches!(self, Self::Redacted { .. })
    }

    pub fn explanation(&self) -> Option<&PolicyExplanation> {
        match self {
            Self::Allow => None,
            Self::Deny { explanation }
            | Self::Restricted { explanation, .. }
            | Self::Redacted { explanation, .. } => Some(explanation),
        }
    }

    pub fn apply<T>(self, value: T) -> PolicyEnforcementOutcome<T> {
        match self {
            Self::Allow => PolicyEnforcementOutcome::Allowed {
                decision: Self::Allow,
                value,
                provenance: None,
                audit: Vec::new(),
            },
            Self::Deny { explanation } => PolicyEnforcementOutcome::Denied {
                decision: Self::Deny { explanation },
                provenance: None,
                audit: Vec::new(),
            },
            Self::Restricted {
                explanation,
                restrictions,
            } => PolicyEnforcementOutcome::Restricted {
                decision: Self::Restricted {
                    explanation,
                    restrictions,
                },
                value,
                provenance: None,
                audit: Vec::new(),
            },
            Self::Redacted {
                explanation,
                redactions,
            } => PolicyEnforcementOutcome::Redacted {
                decision: Self::Redacted {
                    explanation,
                    redactions,
                },
                value,
                provenance: None,
                audit: Vec::new(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvenanceSource {
    Host,
    Adapter,
    Runtime,
    User,
    ExternalPackage,
    Unknown,
}

impl ProvenanceSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Adapter => "adapter",
            Self::Runtime => "runtime",
            Self::User => "user",
            Self::ExternalPackage => "external-package",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceMetadata {
    pub source: ProvenanceSource,
    pub actor: String,
    pub adapter_id: Option<String>,
    pub protocol_family: Option<String>,
    pub target_profile: Option<String>,
    pub runtime: Option<String>,
    pub session_id: Option<String>,
    pub session_ref: Option<String>,
    pub invocation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub details: Vec<String>,
}

impl ProvenanceMetadata {
    pub fn new(source: ProvenanceSource, actor: impl AsRef<str>) -> Result<Self> {
        Ok(Self {
            source,
            actor: trim_required("provenance actor", actor)?,
            adapter_id: None,
            protocol_family: None,
            target_profile: None,
            runtime: None,
            session_id: None,
            session_ref: None,
            invocation_id: None,
            correlation_id: None,
            details: Vec::new(),
        })
    }

    pub fn from_canonical_request(
        request: &CanonicalInvocationRequest,
        source: ProvenanceSource,
    ) -> Result<Self> {
        let mut provenance = Self::new(source, &request.agent.display_name)?;
        provenance.adapter_id = Some(request.agent.agent_key.clone());
        provenance.session_id = Some(request.session.session_id.clone());
        provenance.session_ref = request.session.session_ref.clone();
        provenance.correlation_id = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.correlation_id.clone());
        provenance.invocation_id = provenance.correlation_id.clone();
        provenance.protocol_family = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.protocol_family.clone());
        provenance.target_profile = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.target_profile.clone());
        provenance.runtime = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.runtime.clone());
        provenance
            .details
            .push("derived-from=canonical-invocation-request".to_string());
        Ok(provenance)
    }

    pub fn from_canonical_response(
        response: &CanonicalInvocationResponse,
        source: ProvenanceSource,
    ) -> Result<Self> {
        let mut provenance = Self::new(source, "host")?;
        provenance.session_id = Some(response.session.session_id.clone());
        provenance.session_ref = response.session.session_ref.clone();
        if let Some(lifecycle_event) = &response.lifecycle_event {
            provenance.invocation_id = lifecycle_event
                .tool_use_id
                .as_deref()
                .and_then(trim_optional);
            provenance
                .details
                .push(format!("event-kind={}", lifecycle_event.kind.as_str()));
        }
        provenance
            .details
            .push("derived-from=canonical-invocation-response".to_string());
        Ok(provenance)
    }

    pub fn from_canonical_failure(
        failure: &CanonicalInvocationFailure,
        source: ProvenanceSource,
    ) -> Result<Self> {
        let mut provenance = Self::new(source, "host")?;
        provenance.session_id = failure
            .session
            .as_ref()
            .map(|session| session.session_id.clone());
        provenance.session_ref = failure
            .session
            .as_ref()
            .and_then(|session| session.session_ref.clone());
        if let Some(lifecycle_event) = &failure.lifecycle_event {
            provenance.invocation_id = lifecycle_event
                .tool_use_id
                .as_deref()
                .and_then(trim_optional);
            provenance
                .details
                .push(format!("event-kind={}", lifecycle_event.kind.as_str()));
        }
        provenance
            .details
            .push("derived-from=canonical-invocation-failure".to_string());
        Ok(provenance)
    }

    pub fn with_adapter_id(mut self, adapter_id: impl AsRef<str>) -> Self {
        self.adapter_id = trim_optional(adapter_id);
        self
    }

    pub fn with_protocol_family(mut self, protocol_family: impl AsRef<str>) -> Self {
        self.protocol_family = trim_optional(protocol_family);
        self
    }

    pub fn with_target_profile(mut self, target_profile: impl AsRef<str>) -> Self {
        self.target_profile = trim_optional(target_profile);
        self
    }

    pub fn with_runtime(mut self, runtime: impl AsRef<str>) -> Self {
        self.runtime = trim_optional(runtime);
        self
    }

    pub fn with_session(
        mut self,
        session_id: impl AsRef<str>,
        session_ref: impl AsRef<str>,
    ) -> Self {
        self.session_id = trim_optional(session_id);
        self.session_ref = trim_optional(session_ref);
        self
    }

    pub fn with_invocation(mut self, invocation_id: impl AsRef<str>) -> Self {
        self.invocation_id = trim_optional(invocation_id);
        self
    }

    pub fn with_correlation(mut self, correlation_id: impl AsRef<str>) -> Self {
        self.correlation_id = trim_optional(correlation_id);
        self
    }

    pub fn with_detail(mut self, detail: impl AsRef<str>) -> Self {
        if let Some(detail) = trim_optional(detail) {
            self.details.push(detail);
        }
        self
    }

    pub fn merge(&self, fallback: &Self) -> Self {
        let mut merged = self.clone();
        if merged.adapter_id.is_none() {
            merged.adapter_id = fallback.adapter_id.clone();
        }
        if merged.protocol_family.is_none() {
            merged.protocol_family = fallback.protocol_family.clone();
        }
        if merged.target_profile.is_none() {
            merged.target_profile = fallback.target_profile.clone();
        }
        if merged.runtime.is_none() {
            merged.runtime = fallback.runtime.clone();
        }
        if merged.session_id.is_none() {
            merged.session_id = fallback.session_id.clone();
        }
        if merged.session_ref.is_none() {
            merged.session_ref = fallback.session_ref.clone();
        }
        if merged.invocation_id.is_none() {
            merged.invocation_id = fallback.invocation_id.clone();
        }
        if merged.correlation_id.is_none() {
            merged.correlation_id = fallback.correlation_id.clone();
        }
        merged.details.extend(fallback.details.iter().cloned());
        merged
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditEventKind {
    SessionStarted,
    SessionEnded,
    InvocationStarted,
    InvocationCompleted,
    InvocationDenied,
    InvocationRestricted,
    InvocationRedacted,
    PolicyEvaluated,
}

impl AuditEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session_started",
            Self::SessionEnded => "session_ended",
            Self::InvocationStarted => "invocation_started",
            Self::InvocationCompleted => "invocation_completed",
            Self::InvocationDenied => "invocation_denied",
            Self::InvocationRestricted => "invocation_restricted",
            Self::InvocationRedacted => "invocation_redacted",
            Self::PolicyEvaluated => "policy_evaluated",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRecord {
    pub kind: AuditEventKind,
    pub timestamp: SystemTime,
    pub message: String,
    pub session_id: Option<String>,
    pub session_ref: Option<String>,
    pub invocation_id: Option<String>,
    pub decision: Option<PolicyDecision>,
    pub provenance: Option<ProvenanceMetadata>,
    pub details: Vec<String>,
}

impl AuditRecord {
    pub fn new(kind: AuditEventKind, message: impl AsRef<str>) -> Self {
        Self {
            kind,
            timestamp: SystemTime::now(),
            message: message.as_ref().trim().to_string(),
            session_id: None,
            session_ref: None,
            invocation_id: None,
            decision: None,
            provenance: None,
            details: Vec::new(),
        }
    }

    pub fn for_session(
        kind: AuditEventKind,
        session: &CanonicalSessionDescriptor,
        message: impl AsRef<str>,
    ) -> Self {
        let mut record = Self::new(kind, message);
        record.session_id = trim_optional(&session.session_id);
        record.session_ref = session.session_ref.clone();
        record
    }

    pub fn from_request(
        kind: AuditEventKind,
        request: &CanonicalInvocationRequest,
        message: impl AsRef<str>,
    ) -> Result<Self> {
        let mut record = Self::new(kind, message);
        record.session_id = Some(request.session.session_id.clone());
        record.session_ref = request.session.session_ref.clone();
        record.invocation_id = request
            .correlation
            .as_ref()
            .map(|correlation| correlation.correlation_id.clone());
        if let Some(lifecycle_event) = &request.lifecycle_event {
            record
                .details
                .push(format!("event-kind={}", lifecycle_event.kind.as_str()));
            if record.invocation_id.is_none() {
                record.invocation_id = lifecycle_event
                    .tool_use_id
                    .as_deref()
                    .and_then(trim_optional);
            }
        }
        record.provenance = Some(ProvenanceMetadata::from_canonical_request(
            request,
            ProvenanceSource::Host,
        )?);
        Ok(record)
    }

    pub fn from_response(
        kind: AuditEventKind,
        response: &CanonicalInvocationResponse,
        message: impl AsRef<str>,
    ) -> Result<Self> {
        let mut record = Self::new(kind, message);
        record.session_id = Some(response.session.session_id.clone());
        record.session_ref = response.session.session_ref.clone();
        if let Some(lifecycle_event) = &response.lifecycle_event {
            record
                .details
                .push(format!("event-kind={}", lifecycle_event.kind.as_str()));
            record.invocation_id = lifecycle_event
                .tool_use_id
                .as_deref()
                .and_then(trim_optional);
        }
        record.provenance = Some(ProvenanceMetadata::from_canonical_response(
            response,
            ProvenanceSource::Host,
        )?);
        Ok(record)
    }

    pub fn from_failure(
        kind: AuditEventKind,
        failure: &CanonicalInvocationFailure,
        message: impl AsRef<str>,
    ) -> Result<Self> {
        let mut record = Self::new(kind, message);
        record.session_id = failure
            .session
            .as_ref()
            .map(|session| session.session_id.clone());
        record.session_ref = failure
            .session
            .as_ref()
            .and_then(|session| session.session_ref.clone());
        if let Some(lifecycle_event) = &failure.lifecycle_event {
            record
                .details
                .push(format!("event-kind={}", lifecycle_event.kind.as_str()));
            record.invocation_id = lifecycle_event
                .tool_use_id
                .as_deref()
                .and_then(trim_optional);
        }
        record.provenance = Some(ProvenanceMetadata::from_canonical_failure(
            failure,
            ProvenanceSource::Host,
        )?);
        Ok(record)
    }

    pub fn with_invocation_id(mut self, invocation_id: impl AsRef<str>) -> Self {
        self.invocation_id = trim_optional(invocation_id);
        self
    }

    pub fn with_decision(mut self, decision: PolicyDecision) -> Self {
        self.decision = Some(decision);
        self
    }

    pub fn with_provenance(mut self, provenance: ProvenanceMetadata) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn with_detail(mut self, detail: impl AsRef<str>) -> Self {
        if let Some(detail) = trim_optional(detail) {
            self.details.push(detail);
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyEnforcementOutcome<T> {
    Allowed {
        decision: PolicyDecision,
        value: T,
        provenance: Option<ProvenanceMetadata>,
        audit: Vec<AuditRecord>,
    },
    Restricted {
        decision: PolicyDecision,
        value: T,
        provenance: Option<ProvenanceMetadata>,
        audit: Vec<AuditRecord>,
    },
    Redacted {
        decision: PolicyDecision,
        value: T,
        provenance: Option<ProvenanceMetadata>,
        audit: Vec<AuditRecord>,
    },
    Denied {
        decision: PolicyDecision,
        provenance: Option<ProvenanceMetadata>,
        audit: Vec<AuditRecord>,
    },
}

impl<T> PolicyEnforcementOutcome<T> {
    pub fn decision(&self) -> &PolicyDecision {
        match self {
            Self::Allowed { decision, .. }
            | Self::Restricted { decision, .. }
            | Self::Redacted { decision, .. }
            | Self::Denied { decision, .. } => decision,
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub fn is_restricted(&self) -> bool {
        matches!(self, Self::Restricted { .. })
    }

    pub fn is_redacted(&self) -> bool {
        matches!(self, Self::Redacted { .. })
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    pub fn provenance(&self) -> Option<&ProvenanceMetadata> {
        match self {
            Self::Allowed { provenance, .. }
            | Self::Restricted { provenance, .. }
            | Self::Redacted { provenance, .. }
            | Self::Denied { provenance, .. } => provenance.as_ref(),
        }
    }

    pub fn audit(&self) -> &[AuditRecord] {
        match self {
            Self::Allowed { audit, .. }
            | Self::Restricted { audit, .. }
            | Self::Redacted { audit, .. }
            | Self::Denied { audit, .. } => audit.as_slice(),
        }
    }

    pub fn with_provenance(mut self, provenance: ProvenanceMetadata) -> Self {
        match &mut self {
            Self::Allowed {
                provenance: slot, ..
            }
            | Self::Restricted {
                provenance: slot, ..
            }
            | Self::Redacted {
                provenance: slot, ..
            }
            | Self::Denied {
                provenance: slot, ..
            } => {
                *slot = Some(provenance);
            }
        }
        self
    }

    pub fn with_audit_record(mut self, record: AuditRecord) -> Self {
        match &mut self {
            Self::Allowed { audit, .. }
            | Self::Restricted { audit, .. }
            | Self::Redacted { audit, .. }
            | Self::Denied { audit, .. } => audit.push(record),
        }
        self
    }

    pub fn with_audit_records(mut self, records: Vec<AuditRecord>) -> Self {
        match &mut self {
            Self::Allowed { audit, .. }
            | Self::Restricted { audit, .. }
            | Self::Redacted { audit, .. }
            | Self::Denied { audit, .. } => audit.extend(records),
        }
        self
    }

    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Allowed { value, .. }
            | Self::Restricted { value, .. }
            | Self::Redacted { value, .. } => Some(value),
            Self::Denied { .. } => None,
        }
    }
}

pub fn enforce_policy<T>(
    decision: PolicyDecision,
    value: T,
    provenance: Option<ProvenanceMetadata>,
) -> PolicyEnforcementOutcome<T> {
    match decision {
        PolicyDecision::Allow => PolicyEnforcementOutcome::Allowed {
            decision: PolicyDecision::Allow,
            value,
            provenance,
            audit: Vec::new(),
        },
        PolicyDecision::Deny { explanation } => PolicyEnforcementOutcome::Denied {
            decision: PolicyDecision::Deny { explanation },
            provenance,
            audit: Vec::new(),
        },
        PolicyDecision::Restricted {
            explanation,
            restrictions,
        } => PolicyEnforcementOutcome::Restricted {
            decision: PolicyDecision::Restricted {
                explanation,
                restrictions,
            },
            value,
            provenance,
            audit: Vec::new(),
        },
        PolicyDecision::Redacted {
            explanation,
            redactions,
        } => PolicyEnforcementOutcome::Redacted {
            decision: PolicyDecision::Redacted {
                explanation,
                redactions,
            },
            value,
            provenance,
            audit: Vec::new(),
        },
    }
}

pub fn attach_policy_audit<T>(
    outcome: PolicyEnforcementOutcome<T>,
    record: AuditRecord,
) -> PolicyEnforcementOutcome<T> {
    outcome.with_audit_record(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> CanonicalInvocationRequest {
        let session = CanonicalSessionDescriptor::new("session-42")
            .expect("session")
            .with_session_ref("/tmp/session-42.jsonl");
        CanonicalInvocationRequest::new(
            super::super::canonical::CanonicalAgentIdentity::new("Claude Code", "Claude Code")
                .expect("identity"),
            session,
        )
        .with_correlation(super::super::canonical::CanonicalCorrelationMetadata {
            correlation_id: "corr-42".to_string(),
            protocol_family: "jsonl-cli".to_string(),
            target_profile: "claude-code".to_string(),
            runtime: "local-cli".to_string(),
            resolution_path: "legacy-target-compat".to_string(),
        })
        .with_prompt("  analyse this  ")
    }

    #[test]
    fn policy_decisions_classify_and_apply_deterministically() {
        let allowed = PolicyDecision::allow();
        assert!(allowed.is_allowed());
        assert!(allowed.explanation().is_none());

        let denied =
            PolicyDecision::deny("policy.blocked", "  forbidden operation  ").expect("decision");
        assert!(denied.is_denied());
        assert_eq!(
            denied.explanation().expect("explanation").code,
            "policy.blocked"
        );
        assert_eq!(
            denied.explanation().expect("explanation").reason,
            "forbidden operation"
        );

        let restrictions =
            vec![PolicyRestriction::new("session", "read-only mode").expect("restriction")];
        let restricted = PolicyDecision::restricted(
            "policy.restricted",
            "  limited execution  ",
            restrictions.clone(),
        )
        .expect("decision");
        assert!(restricted.is_restricted());
        assert_eq!(
            match &restricted {
                PolicyDecision::Restricted { restrictions, .. } => restrictions,
                _ => panic!("expected restricted"),
            },
            &restrictions
        );

        let redactions = vec![PolicyRedaction::new("prompt", "[hidden]").expect("redaction")];
        let redacted = PolicyDecision::redacted(
            "policy.redacted",
            "  sensitive content  ",
            redactions.clone(),
        )
        .expect("decision");
        assert!(redacted.is_redacted());
        assert_eq!(
            match &redacted {
                PolicyDecision::Redacted { redactions, .. } => redactions,
                _ => panic!("expected redacted"),
            },
            &redactions
        );

        let allowed_outcome = PolicyDecision::allow().apply("payload".to_string());
        assert!(allowed_outcome.is_allowed());
        assert_eq!(allowed_outcome.into_value().as_deref(), Some("payload"));

        let denied_outcome = denied.clone().apply("payload".to_string());
        assert!(denied_outcome.is_denied());
        assert!(denied_outcome.into_value().is_none());

        let restricted_outcome = restricted.clone().apply("payload".to_string());
        assert!(restricted_outcome.is_restricted());
        assert_eq!(restricted_outcome.decision(), &restricted);
        assert_eq!(restricted_outcome.into_value().as_deref(), Some("payload"));

        let redacted_outcome = redacted.clone().apply("payload".to_string());
        assert!(redacted_outcome.is_redacted());
        assert_eq!(redacted_outcome.decision(), &redacted);
        assert_eq!(redacted_outcome.into_value().as_deref(), Some("payload"));
    }

    #[test]
    fn provenance_derives_from_canonical_requests_and_merges_context() {
        let request = sample_request();
        let provenance =
            ProvenanceMetadata::from_canonical_request(&request, ProvenanceSource::Host)
                .expect("provenance");
        assert_eq!(provenance.source, ProvenanceSource::Host);
        assert_eq!(provenance.actor, "Claude Code");
        assert_eq!(provenance.adapter_id.as_deref(), Some("claude-code"));
        assert_eq!(provenance.protocol_family.as_deref(), Some("jsonl-cli"));
        assert_eq!(provenance.target_profile.as_deref(), Some("claude-code"));
        assert_eq!(provenance.runtime.as_deref(), Some("local-cli"));
        assert_eq!(provenance.session_id.as_deref(), Some("session-42"));
        assert_eq!(
            provenance.session_ref.as_deref(),
            Some("/tmp/session-42.jsonl")
        );
        assert_eq!(provenance.invocation_id.as_deref(), Some("corr-42"));
        assert_eq!(provenance.correlation_id.as_deref(), Some("corr-42"));

        let fallback = ProvenanceMetadata::new(ProvenanceSource::Adapter, "adapter")
            .expect("fallback")
            .with_adapter_id("adapter-x")
            .with_detail("fallback");
        let merged = ProvenanceMetadata::new(ProvenanceSource::User, "user")
            .expect("merged")
            .merge(&fallback);
        assert_eq!(merged.adapter_id.as_deref(), Some("adapter-x"));
        assert!(merged.details.iter().any(|detail| detail == "fallback"));
    }

    #[test]
    fn audit_records_capture_session_and_invocation_context() {
        let request = sample_request();
        let response = CanonicalInvocationResponse::new(
            CanonicalSessionDescriptor::new("session-42")
                .expect("session")
                .with_session_ref("/tmp/session-42.jsonl"),
        )
        .with_output("done");
        let failure = CanonicalInvocationFailure::new(
            super::super::canonical::CanonicalInvocationFailureKind::Transient,
            "temporary failure",
        )
        .with_session(
            CanonicalSessionDescriptor::new("session-42")
                .expect("session")
                .with_session_ref("/tmp/session-42.jsonl"),
        );
        let request_audit = AuditRecord::from_request(
            AuditEventKind::InvocationStarted,
            &request,
            "starting invocation",
        )
        .expect("request audit");
        assert_eq!(request_audit.kind, AuditEventKind::InvocationStarted);
        assert_eq!(request_audit.session_id.as_deref(), Some("session-42"));
        assert_eq!(request_audit.invocation_id.as_deref(), Some("corr-42"));
        assert!(request_audit.provenance.is_some());

        let response_audit = AuditRecord::from_response(
            AuditEventKind::InvocationCompleted,
            &response,
            "completed invocation",
        )
        .expect("response audit");
        assert_eq!(
            response_audit.session_ref.as_deref(),
            Some("/tmp/session-42.jsonl")
        );
        assert!(response_audit.provenance.is_some());

        let failure_audit =
            AuditRecord::from_failure(AuditEventKind::InvocationDenied, &failure, "denied")
                .expect("failure audit");
        assert!(failure_audit.provenance.is_some());

        let session_audit = AuditRecord::for_session(
            AuditEventKind::SessionStarted,
            &request.session,
            "session started",
        );
        assert_eq!(session_audit.session_id.as_deref(), Some("session-42"));
    }

    #[test]
    fn enforcement_helpers_attach_provenance_and_audit() {
        let request = sample_request();
        let provenance =
            ProvenanceMetadata::from_canonical_request(&request, ProvenanceSource::Host)
                .expect("provenance");
        let decision = PolicyDecision::redacted(
            "policy.redacted",
            "sensitive prompt",
            vec![PolicyRedaction::new("prompt", "[redacted]").expect("redaction")],
        )
        .expect("decision");
        let outcome = enforce_policy(decision, "payload".to_string(), Some(provenance.clone()))
            .with_audit_record(
                AuditRecord::new(AuditEventKind::PolicyEvaluated, "policy evaluated")
                    .with_provenance(provenance.clone()),
            );

        assert!(outcome.is_redacted());
        assert_eq!(outcome.provenance(), Some(&provenance));
        assert_eq!(outcome.audit().len(), 1);

        let denied = PolicyDecision::deny("policy.denied", "blocked").expect("decision");
        let denied_outcome = attach_policy_audit(
            enforce_policy(denied, "payload".to_string(), None),
            AuditRecord::new(AuditEventKind::InvocationDenied, "blocked"),
        );
        assert!(denied_outcome.is_denied());
        assert_eq!(denied_outcome.audit().len(), 1);
    }
}
