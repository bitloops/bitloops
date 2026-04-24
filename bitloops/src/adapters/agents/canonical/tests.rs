use crate::host::checkpoints::lifecycle::{LifecycleEvent, LifecycleEventType};

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
fn canonical_contract_versions_and_compatibility_are_explicit() {
    let current = CanonicalContractVersion::current();
    assert_eq!(current, CanonicalContractVersion::new(1, 0, 0));
    assert!(current.is_compatible_with(CanonicalContractVersion::new(1, 0, 0)));
    assert!(!current.is_compatible_with(CanonicalContractVersion::new(2, 0, 0)));

    let simple = CanonicalContractCompatibility::default();
    assert_eq!(simple.version, current);
    assert!(!simple.is_rich());
    assert!(simple.is_compatible_with(CanonicalContractVersion::new(1, 0, 0)));

    let rich = CanonicalContractCompatibility::rich();
    assert!(rich.is_rich());
    assert!(rich.supports_streaming);
    assert!(rich.supports_progress);
    assert!(rich.supports_partial_results);
    assert!(rich.supports_resumable_sessions);
}

#[test]
fn progress_stream_and_partial_results_builders_preserve_rich_state() {
    let session = CanonicalSessionDescriptor::new("session-progress")
        .expect("session")
        .with_session_ref("/tmp/session-progress.jsonl");
    let progress = CanonicalProgressUpdate::new()
        .with_label("Indexing")
        .with_message("three items complete")
        .with_counts(3, 10);
    assert_eq!(progress.completed, Some(3));
    assert_eq!(progress.total, Some(10));
    assert_eq!(progress.percentage, Some(30));
    assert!(!progress.is_complete());

    let resumable_session = CanonicalResumableSession::new(session.clone())
        .with_checkpoint("/tmp/checkpoint.json")
        .with_resume_token("resume-token-1")
        .with_last_sequence(12)
        .mark_resumable();
    assert!(resumable_session.can_resume());
    assert!(!resumable_session.is_terminal());

    let partial_result = CanonicalResultFragment::partial("chunk 1")
        .with_reason("waiting for more output")
        .with_resumable_session(resumable_session.clone());
    assert!(partial_result.is_partial());
    assert!(!partial_result.is_final());
    assert_eq!(
        partial_result.reason.as_deref(),
        Some("waiting for more output")
    );
    assert_eq!(
        partial_result.resumable_session.as_ref(),
        Some(&resumable_session)
    );

    let stream_event = CanonicalStreamEvent::progress(session.clone(), progress.clone())
        .with_sequence(7)
        .with_message("indexing in progress");
    assert_eq!(stream_event.kind, CanonicalStreamEventKind::Progress);
    assert_eq!(stream_event.sequence, 7);
    assert_eq!(stream_event.progress.as_ref(), Some(&progress));
    assert_eq!(
        stream_event.message.as_deref(),
        Some("indexing in progress")
    );
    assert!(!stream_event.is_terminal());

    let response = CanonicalInvocationResponse::new(session.clone())
        .with_stream_event(stream_event)
        .with_result_fragment(partial_result.clone())
        .with_progress(progress.clone())
        .with_resumable_session(resumable_session.clone());
    assert_eq!(response.stream_events.len(), 1);
    assert_eq!(response.progress.as_ref(), Some(&progress));
    assert_eq!(response.result_fragment.as_ref(), Some(&partial_result));
    assert_eq!(
        response.resumable_session.as_ref(),
        Some(&resumable_session)
    );
}

#[test]
fn resumable_session_helpers_track_state_transitions() {
    let session = CanonicalSessionDescriptor::new("session-resume").expect("session");
    let suspended = CanonicalResumableSession::new(session.clone())
        .with_checkpoint("/tmp/checkpoint.json")
        .with_note("waiting for user input")
        .mark_suspended();
    assert_eq!(suspended.state, CanonicalResumableSessionState::Suspended);
    assert!(suspended.can_resume());
    assert!(!suspended.is_terminal());

    let resumable = suspended.clone().mark_resumable();
    assert_eq!(resumable.state, CanonicalResumableSessionState::Resumable);
    assert!(resumable.can_resume());

    let resumed = resumable.clone().mark_resumed().with_last_sequence(99);
    assert_eq!(resumed.state, CanonicalResumableSessionState::Resumed);
    assert_eq!(resumed.last_sequence, Some(99));
    assert!(!resumed.can_resume());

    let completed = resumed.mark_completed();
    assert_eq!(completed.state, CanonicalResumableSessionState::Completed);
    assert!(completed.is_terminal());

    let deferred = CanonicalResultFragment::deferred("session paused")
        .with_resumable_session(completed.clone());
    assert!(deferred.is_deferred());
    assert!(deferred.is_terminal());
    assert_eq!(deferred.resumable_session.as_ref(), Some(&completed));
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
        source: String::new(),
        prompt: "  hello world  ".to_string(),
        tool_name: String::new(),
        tool_use_id: "  tool-1  ".to_string(),
        tool_input: None,
        tool_response: None,
        subagent_id: "  subagent-9  ".to_string(),
        model: "  gemini-2.5  ".to_string(),
        finalize_open_turn: false,
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

    let request =
        CanonicalInvocationRequest::for_lifecycle_event("Claude Code", &event).expect("request");
    assert_eq!(request.agent.agent_key, "claude-code");
    assert_eq!(request.agent.display_name, "Claude Code");
    assert_eq!(request.session.session_id, "session-42");
    assert_eq!(
        request.session.session_ref.as_deref(),
        Some("/tmp/session-42.jsonl")
    );
    assert_eq!(request.prompt.as_deref(), Some("ping"));
    assert_eq!(request.tool_use_id.as_deref(), Some("tool-use-42"));
    assert_eq!(
        request.compatibility,
        CanonicalContractCompatibility::default()
    );
    assert!(request.progress.is_none());
    assert!(request.resumable_session.is_none());
    assert!(request.lifecycle_event.is_some());
    assert!(request.correlation.is_some());
    let correlation = request.correlation.as_ref().expect("correlation");
    assert_eq!(correlation.protocol_family, "jsonl-cli");
    assert_eq!(correlation.target_profile, "claude-code");
    assert!(!correlation.correlation_id.is_empty());
    assert_eq!(request.capabilities, HostCapabilityFlags::default());
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
        .with_result_fragment(CanonicalResultFragment::final_output("  done  "))
        .with_progress(CanonicalProgressUpdate::new().with_percentage(100))
        .with_modified_files(vec!["src/main.rs".to_string()]);
    assert_eq!(response.session, session);
    assert_eq!(response.output.as_deref(), Some("done"));
    assert_eq!(
        response.result_fragment.as_ref(),
        Some(&CanonicalResultFragment::final_output("done"))
    );
    assert_eq!(
        response.progress.as_ref(),
        Some(&CanonicalProgressUpdate::new().with_percentage(100))
    );
    assert_eq!(response.modified_files, vec!["src/main.rs".to_string()]);
    assert_eq!(response.lifecycle_event.as_ref(), Some(&lifecycle_event));

    let failure = CanonicalInvocationFailure::new(
        CanonicalInvocationFailureKind::Transient,
        "  temporary failure  ",
    )
    .with_session(session.clone())
    .with_lifecycle_event(lifecycle_event)
    .with_progress(CanonicalProgressUpdate::new().with_counts(5, 10))
    .with_resumable_session(
        CanonicalResumableSession::new(session.clone())
            .with_resume_token("resume-token")
            .mark_resumable(),
    );
    assert_eq!(failure.session.as_ref(), Some(&session));
    assert_eq!(failure.message, "temporary failure");
    assert!(failure.retryable);
    assert_eq!(
        failure.progress.as_ref(),
        Some(&CanonicalProgressUpdate::new().with_counts(5, 10))
    );
    assert!(
        failure
            .resumable_session
            .as_ref()
            .is_some_and(CanonicalResumableSession::can_resume)
    );
}
