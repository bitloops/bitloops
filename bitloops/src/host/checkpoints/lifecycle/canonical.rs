use anyhow::Result;

use crate::adapters::agents::canonical::{
    CanonicalContractCompatibility, CanonicalInvocationRequest, CanonicalProgressUpdate,
    CanonicalResumableSession,
};
use crate::adapters::agents::{AgentAdapterCapability, AgentAdapterRegistry};

use super::types::LifecycleEvent;

pub(crate) fn build_phase3_canonical_request(
    agent_name: &str,
    event: &LifecycleEvent,
) -> Result<CanonicalInvocationRequest> {
    let request = CanonicalInvocationRequest::for_lifecycle_event(agent_name, event)?;
    Ok(enrich_phase3_canonical_request(request))
}

pub(super) fn enrich_phase3_canonical_request(
    request: CanonicalInvocationRequest,
) -> CanonicalInvocationRequest {
    let Ok(resolved) =
        AgentAdapterRegistry::builtin().resolve_with_trace(&request.agent.agent_key, None)
    else {
        return request;
    };

    let descriptor = resolved.registration.descriptor();
    let supports_richer_contract = descriptor
        .capabilities
        .contains(&AgentAdapterCapability::TranscriptAnalysis)
        || descriptor
            .capabilities
            .contains(&AgentAdapterCapability::TokenCalculation);

    if !supports_richer_contract {
        return request.with_compatibility(CanonicalContractCompatibility::simple());
    }

    let session_id = request.session.session_id.clone();
    let session_ref = request.session.session_ref.clone().unwrap_or_default();
    let resumable_session = CanonicalResumableSession::new(request.session.clone())
        .with_checkpoint(session_ref)
        .with_resume_token(session_id.as_str())
        .with_note(format!(
            "{}:{}",
            descriptor.protocol_family.id, descriptor.target_profile.id
        ))
        .mark_resumable();

    request
        .with_compatibility(CanonicalContractCompatibility::rich())
        .with_progress(
            CanonicalProgressUpdate::new()
                .with_label(descriptor.display_name)
                .with_message("rich canonical lifecycle semantics enabled"),
        )
        .with_resumable_session(resumable_session)
}
