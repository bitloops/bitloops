use super::*;

struct TransitionCase {
    current: SessionPhase,
    event: Event,
    context: TransitionContext,
    expected_phase: SessionPhase,
    expected_actions: Vec<Action>,
}

struct MockActionHandler {
    condense_called: bool,
    condense_if_files_touched_called: bool,
    discard_if_no_files_called: bool,
    warn_stale_session_called: bool,
    return_error: Option<String>,
}

impl MockActionHandler {
    fn new(return_error: Option<&str>) -> Self {
        Self {
            condense_called: false,
            condense_if_files_touched_called: false,
            discard_if_no_files_called: false,
            warn_stale_session_called: false,
            return_error: return_error.map(ToOwned::to_owned),
        }
    }
}

impl ActionHandler for MockActionHandler {
    fn handle_condense(&mut self, _state: &mut SessionState) -> Result<()> {
        self.condense_called = true;
        match &self.return_error {
            Some(err) => Err(anyhow::anyhow!("{}", err)),
            None => Ok(()),
        }
    }

    fn handle_condense_if_files_touched(&mut self, _state: &mut SessionState) -> Result<()> {
        self.condense_if_files_touched_called = true;
        match &self.return_error {
            Some(err) => Err(anyhow::anyhow!("{}", err)),
            None => Ok(()),
        }
    }

    fn handle_discard_if_no_files(&mut self, _state: &mut SessionState) -> Result<()> {
        self.discard_if_no_files_called = true;
        match &self.return_error {
            Some(err) => Err(anyhow::anyhow!("{}", err)),
            None => Ok(()),
        }
    }

    fn handle_warn_stale_session(&mut self, _state: &mut SessionState) -> Result<()> {
        self.warn_stale_session_called = true;
        match &self.return_error {
            Some(err) => Err(anyhow::anyhow!("{}", err)),
            None => Ok(()),
        }
    }
}

fn run_transition_cases(cases: &[TransitionCase]) {
    for case in cases {
        let result = transition_with_context(case.current, case.event, case.context);
        assert_eq!(result.new_phase, case.expected_phase, "unexpected phase");
        assert_eq!(result.actions, case.expected_actions, "unexpected actions");
    }
}

fn make_state(phase: SessionPhase) -> SessionState {
    SessionState {
        phase,
        ..SessionState::default()
    }
}

// CLI-237
#[test]
fn test_phase_from_string() {
    let cases = [
        ("active", SessionPhase::Active),
        ("active_committed", SessionPhase::Active),
        ("idle", SessionPhase::Idle),
        ("ended", SessionPhase::Ended),
        ("", SessionPhase::Idle),
        ("bogus", SessionPhase::Idle),
        ("ACTIVE", SessionPhase::Idle),
    ];

    for (input, expected) in cases {
        assert_eq!(SessionPhase::from_string(input), expected);
    }
}

#[test]
fn test_phase_deserialize_non_string_and_null_defaults_to_idle() {
    for input in ["null", "true", "1", "[]", "{}"] {
        let phase: SessionPhase = serde_json::from_str(input).expect("phase should deserialize");
        assert_eq!(phase, SessionPhase::Idle, "{input}");
    }
}

// CLI-238
#[test]
fn test_phase_is_active() {
    let cases = [
        (SessionPhase::Active, true),
        (SessionPhase::Idle, false),
        (SessionPhase::Ended, false),
    ];

    for (phase, expected) in cases {
        assert_eq!(phase.is_active(), expected);
    }
}

// CLI-239
#[test]
fn test_event_string() {
    let cases = [
        (Event::TurnStart, "TurnStart"),
        (Event::TurnEnd, "TurnEnd"),
        (Event::GitCommit, "GitCommit"),
        (Event::SessionStart, "SessionStart"),
        (Event::SessionStop, "SessionStop"),
    ];

    for (event, expected) in cases {
        assert_eq!(event.to_string(), expected);
    }
}

// CLI-240
#[test]
fn test_action_string() {
    let cases = [
        (Action::Condense, "Condense"),
        (Action::CondenseIfFilesTouched, "CondenseIfFilesTouched"),
        (Action::DiscardIfNoFiles, "DiscardIfNoFiles"),
        (Action::WarnStaleSession, "WarnStaleSession"),
        (Action::ClearEndedAt, "ClearEndedAt"),
        (Action::UpdateLastInteraction, "UpdateLastInteraction"),
    ];

    for (action, expected) in cases {
        assert_eq!(action.to_string(), expected);
    }
}

// CLI-241
#[test]
fn test_transition_from_idle() {
    run_transition_cases(&[
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::TurnStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::GitCommit,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: vec![Action::Condense, Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::GitCommit,
            context: TransitionContext {
                is_rebase_in_progress: true,
                ..TransitionContext::default()
            },
            expected_phase: SessionPhase::Idle,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::SessionStop,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::SessionStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::Idle,
            event: Event::TurnEnd,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: Vec::new(),
        },
    ]);
}

// CLI-242
#[test]
fn test_transition_from_active() {
    run_transition_cases(&[
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::TurnStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::TurnEnd,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::GitCommit,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::Condense, Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::GitCommit,
            context: TransitionContext {
                is_rebase_in_progress: true,
                ..TransitionContext::default()
            },
            expected_phase: SessionPhase::Active,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::SessionStop,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Active,
            event: Event::SessionStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::WarnStaleSession],
        },
    ]);
}

// CLI-243
#[test]
fn test_transition_from_ended() {
    run_transition_cases(&[
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::TurnStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::ClearEndedAt, Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::GitCommit,
            context: TransitionContext {
                has_files_touched: true,
                ..TransitionContext::default()
            },
            expected_phase: SessionPhase::Ended,
            expected_actions: vec![
                Action::CondenseIfFilesTouched,
                Action::UpdateLastInteraction,
            ],
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::GitCommit,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: vec![Action::DiscardIfNoFiles, Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::GitCommit,
            context: TransitionContext {
                is_rebase_in_progress: true,
                ..TransitionContext::default()
            },
            expected_phase: SessionPhase::Ended,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::SessionStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: vec![Action::ClearEndedAt],
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::TurnEnd,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::Ended,
            event: Event::SessionStop,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: Vec::new(),
        },
    ]);
}

// CLI-244
#[test]
fn test_transition_backward_compat() {
    run_transition_cases(&[
        TransitionCase {
            current: SessionPhase::from_string(""),
            event: Event::TurnStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::from_string(""),
            event: Event::GitCommit,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: vec![Action::Condense, Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::from_string(""),
            event: Event::SessionStop,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Ended,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
        TransitionCase {
            current: SessionPhase::from_string(""),
            event: Event::SessionStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::from_string(""),
            event: Event::TurnEnd,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Idle,
            expected_actions: Vec::new(),
        },
        TransitionCase {
            current: SessionPhase::from_string("bogus"),
            event: Event::TurnStart,
            context: TransitionContext::default(),
            expected_phase: SessionPhase::Active,
            expected_actions: vec![Action::UpdateLastInteraction],
        },
    ]);
}

// CLI-245
#[test]
fn test_transition_rebase_git_commit_is_noop_for_all_phases() {
    let rebase_context = TransitionContext {
        is_rebase_in_progress: true,
        ..TransitionContext::default()
    };

    for phase in all_phases() {
        let result = transition_with_context(*phase, Event::GitCommit, rebase_context);
        assert!(
            result.actions.is_empty(),
            "expected no actions for phase {}",
            phase.as_str()
        );
        assert_eq!(
            result.new_phase,
            *phase,
            "phase changed for {}",
            phase.as_str()
        );
    }
}

// CLI-246
#[test]
fn test_transition_all_phase_event_combinations_are_defined() {
    for phase in all_phases() {
        for event in all_events() {
            let result = transition_with_context(*phase, *event, TransitionContext::default());
            let normalized = SessionPhase::from_string(result.new_phase.as_str());
            assert_eq!(
                result.new_phase,
                normalized,
                "transition returned non-canonical phase for phase {} and event {}",
                phase.as_str(),
                event
            );
        }
    }
}

// CLI-247
#[test]
fn test_mermaid_diagram() {
    let diagram = mermaid_diagram();

    assert!(diagram.contains("stateDiagram-v2"));
    assert!(diagram.contains("IDLE"));
    assert!(diagram.contains("ACTIVE"));
    assert!(diagram.contains("ENDED"));
    assert!(!diagram.contains("ACTIVE_COMMITTED"));

    assert!(diagram.contains("idle --> active"));
    assert!(diagram.contains("active --> active"));
    assert!(diagram.contains("ended --> idle"));
    assert!(diagram.contains("ended --> active"));

    assert!(diagram.contains("Condense"));
    assert!(diagram.contains("ClearEndedAt"));
    assert!(diagram.contains("WarnStaleSession"));
    assert!(!diagram.contains("MigrateShadowBranch"));
}

// CLI-248
#[test]
fn test_apply_transition_sets_phase_and_handles_common_actions() {
    let mut state = make_state(SessionPhase::Idle);
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Active,
        actions: vec![Action::UpdateLastInteraction],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert_eq!(state.phase, SessionPhase::Active);
    assert!(state.last_interaction_time.is_some());
    assert!(!handler.condense_called);
}

// CLI-249
#[test]
fn test_apply_transition_calls_handler_for_condense() {
    let mut state = make_state(SessionPhase::Active);
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Idle,
        actions: vec![Action::Condense, Action::UpdateLastInteraction],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert!(handler.condense_called);
    assert_eq!(state.phase, SessionPhase::Idle);
    assert!(state.last_interaction_time.is_some());
}

// CLI-250
#[test]
fn test_apply_transition_calls_handler_for_condense_if_files_touched() {
    let mut state = make_state(SessionPhase::Ended);
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Ended,
        actions: vec![
            Action::CondenseIfFilesTouched,
            Action::UpdateLastInteraction,
        ],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert!(handler.condense_if_files_touched_called);
}

// CLI-251
#[test]
fn test_apply_transition_calls_handler_for_discard_if_no_files() {
    let mut state = make_state(SessionPhase::Ended);
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Ended,
        actions: vec![Action::DiscardIfNoFiles, Action::UpdateLastInteraction],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert!(handler.discard_if_no_files_called);
}

// CLI-252
#[test]
fn test_apply_transition_calls_handler_for_warn_stale_session() {
    let mut state = make_state(SessionPhase::Active);
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Active,
        actions: vec![Action::WarnStaleSession],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert!(handler.warn_stale_session_called);
}

// CLI-253
#[test]
fn test_apply_transition_clears_ended_at() {
    let mut state = make_state(SessionPhase::Ended);
    state.ended_at = Some("2026-02-21T10:00:00Z".to_string());
    let mut handler = MockActionHandler::new(None);
    let result = TransitionResult {
        new_phase: SessionPhase::Idle,
        actions: vec![Action::ClearEndedAt],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_ok());
    assert!(state.ended_at.is_none());
}

// CLI-254
#[test]
fn test_apply_transition_returns_handler_error_but_runs_common_actions() {
    let mut state = make_state(SessionPhase::Active);
    let mut handler = MockActionHandler::new(Some("condense failed"));
    let result = TransitionResult {
        new_phase: SessionPhase::Idle,
        actions: vec![Action::Condense, Action::UpdateLastInteraction],
    };

    let err = apply_transition(&mut state, result, &mut handler).expect_err("expected error");
    let msg = err.to_string();
    assert!(msg.contains("Condense"), "missing action name, got: {msg}");
    assert!(
        msg.contains("condense failed"),
        "missing handler error, got: {msg}"
    );
    assert_eq!(state.phase, SessionPhase::Idle);
    assert!(state.last_interaction_time.is_some());
}

// CLI-255
#[test]
fn test_apply_transition_stops_on_first_handler_error() {
    let mut state = make_state(SessionPhase::Active);
    let mut handler = MockActionHandler::new(Some("condense failed"));
    let result = TransitionResult {
        new_phase: SessionPhase::Idle,
        actions: vec![Action::Condense, Action::WarnStaleSession],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_err());
    assert!(handler.condense_called);
    assert!(!handler.warn_stale_session_called);
}

// CLI-256
#[test]
fn test_apply_transition_clear_ended_at_runs_despite_handler_error() {
    let mut state = make_state(SessionPhase::Ended);
    state.ended_at = Some("2026-02-21T10:00:00Z".to_string());
    let mut handler = MockActionHandler::new(Some("condense failed"));
    let result = TransitionResult {
        new_phase: SessionPhase::Ended,
        actions: vec![Action::CondenseIfFilesTouched, Action::ClearEndedAt],
    };

    let apply_result = apply_transition(&mut state, result, &mut handler);

    assert!(apply_result.is_err());
    assert!(state.ended_at.is_none());
}
