//! Session lifecycle phase enum and state-machine transition function.

use std::fmt;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::state::SessionState;

/// Lifecycle stage of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    #[default]
    Idle,
    Active,
    Ended,
}

impl SessionPhase {
    pub fn from_string(value: &str) -> Self {
        match value {
            "active" | "active_committed" => Self::Active,
            "idle" => Self::Idle,
            "ended" => Self::Ended,
            _ => Self::Idle,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Ended => "ended",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

impl<'de> Deserialize<'de> for SessionPhase {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SessionPhaseVisitor;

        impl<'de> serde::de::Visitor<'de> for SessionPhaseVisitor {
            type Value = SessionPhase;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a session phase string or null")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::from_string(value))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::from_string(&value))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(self)
            }

            fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_seq<A>(self, _seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                Ok(SessionPhase::Idle)
            }

            fn visit_map<A>(self, _map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                Ok(SessionPhase::Idle)
            }
        }

        deserializer.deserialize_any(SessionPhaseVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    TurnStart,
    TurnEnd,
    GitCommit,
    SessionStart,
    SessionStop,
    Compaction,
}

impl Event {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TurnStart => "TurnStart",
            Self::TurnEnd => "TurnEnd",
            Self::GitCommit => "GitCommit",
            Self::SessionStart => "SessionStart",
            Self::SessionStop => "SessionStop",
            Self::Compaction => "Compaction",
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Condense,
    CondenseIfFilesTouched,
    DiscardIfNoFiles,
    WarnStaleSession,
    ClearEndedAt,
    UpdateLastInteraction,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Condense => "Condense",
            Self::CondenseIfFilesTouched => "CondenseIfFilesTouched",
            Self::DiscardIfNoFiles => "DiscardIfNoFiles",
            Self::WarnStaleSession => "WarnStaleSession",
            Self::ClearEndedAt => "ClearEndedAt",
            Self::UpdateLastInteraction => "UpdateLastInteraction",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransitionContext {
    pub has_files_touched: bool,
    pub is_rebase_in_progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionResult {
    pub new_phase: SessionPhase,
    pub actions: Vec<Action>,
}

#[cfg(test)]
pub fn all_phases() -> &'static [SessionPhase] {
    &[
        SessionPhase::Idle,
        SessionPhase::Active,
        SessionPhase::Ended,
    ]
}

#[cfg(test)]
pub fn all_events() -> &'static [Event] {
    &[
        Event::TurnStart,
        Event::TurnEnd,
        Event::GitCommit,
        Event::SessionStart,
        Event::SessionStop,
        Event::Compaction,
    ]
}

pub fn transition_with_context(
    current: SessionPhase,
    event: Event,
    ctx: TransitionContext,
) -> TransitionResult {
    let normalized = SessionPhase::from_string(current.as_str());

    match normalized {
        SessionPhase::Idle => transition_from_idle(event, ctx),
        SessionPhase::Active => transition_from_active(event, ctx),
        SessionPhase::Ended => transition_from_ended(event, ctx),
    }
}

fn transition_from_idle(event: Event, ctx: TransitionContext) -> TransitionResult {
    match event {
        Event::TurnStart => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![Action::UpdateLastInteraction],
        },
        Event::TurnEnd => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: Vec::new(),
        },
        Event::GitCommit if ctx.is_rebase_in_progress => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: Vec::new(),
        },
        Event::GitCommit => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: vec![Action::Condense, Action::UpdateLastInteraction],
        },
        Event::SessionStart => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: Vec::new(),
        },
        Event::SessionStop => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: vec![Action::UpdateLastInteraction],
        },
        Event::Compaction => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: vec![
                Action::CondenseIfFilesTouched,
                Action::UpdateLastInteraction,
            ],
        },
    }
}

fn transition_from_active(event: Event, ctx: TransitionContext) -> TransitionResult {
    match event {
        Event::TurnStart => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![Action::UpdateLastInteraction],
        },
        Event::TurnEnd => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: vec![Action::UpdateLastInteraction],
        },
        Event::GitCommit if ctx.is_rebase_in_progress => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: Vec::new(),
        },
        Event::GitCommit => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![Action::Condense, Action::UpdateLastInteraction],
        },
        Event::SessionStart => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![Action::WarnStaleSession],
        },
        Event::SessionStop => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: vec![Action::UpdateLastInteraction],
        },
        Event::Compaction => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![
                Action::CondenseIfFilesTouched,
                Action::UpdateLastInteraction,
            ],
        },
    }
}

fn transition_from_ended(event: Event, ctx: TransitionContext) -> TransitionResult {
    match event {
        Event::TurnStart => TransitionResult {
            new_phase: SessionPhase::Active,
            actions: vec![Action::ClearEndedAt, Action::UpdateLastInteraction],
        },
        Event::TurnEnd => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: Vec::new(),
        },
        Event::GitCommit if ctx.is_rebase_in_progress => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: Vec::new(),
        },
        Event::GitCommit if ctx.has_files_touched => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: vec![
                Action::CondenseIfFilesTouched,
                Action::UpdateLastInteraction,
            ],
        },
        Event::GitCommit => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: vec![Action::DiscardIfNoFiles, Action::UpdateLastInteraction],
        },
        Event::SessionStart => TransitionResult {
            new_phase: SessionPhase::Idle,
            actions: vec![Action::ClearEndedAt],
        },
        Event::SessionStop => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: Vec::new(),
        },
        Event::Compaction => TransitionResult {
            new_phase: SessionPhase::Ended,
            actions: Vec::new(),
        },
    }
}

pub trait ActionHandler {
    fn handle_condense(&mut self, state: &mut SessionState) -> Result<()>;
    fn handle_condense_if_files_touched(&mut self, state: &mut SessionState) -> Result<()>;
    fn handle_discard_if_no_files(&mut self, state: &mut SessionState) -> Result<()>;
    fn handle_warn_stale_session(&mut self, state: &mut SessionState) -> Result<()>;
}

pub struct NoOpActionHandler;

impl ActionHandler for NoOpActionHandler {
    fn handle_condense(&mut self, _state: &mut SessionState) -> Result<()> {
        Ok(())
    }

    fn handle_condense_if_files_touched(&mut self, _state: &mut SessionState) -> Result<()> {
        Ok(())
    }

    fn handle_discard_if_no_files(&mut self, _state: &mut SessionState) -> Result<()> {
        Ok(())
    }

    fn handle_warn_stale_session(&mut self, _state: &mut SessionState) -> Result<()> {
        Ok(())
    }
}

fn record_first_error<F>(slot: &mut Option<anyhow::Error>, action: Action, f: F)
where
    F: FnOnce() -> Result<()>,
{
    if slot.is_none()
        && let Err(err) = f()
    {
        *slot = Some(anyhow::anyhow!("{action}: {err}"));
    }
}

pub fn apply_transition(
    state: &mut SessionState,
    result: TransitionResult,
    handler: &mut dyn ActionHandler,
) -> Result<()> {
    state.phase = result.new_phase;

    let mut first_error: Option<anyhow::Error> = None;
    for action in result.actions {
        match action {
            Action::UpdateLastInteraction => {
                state.last_interaction_time = Some(now_rfc3339());
            }
            Action::ClearEndedAt => {
                state.ended_at = None;
            }
            Action::Condense => {
                record_first_error(&mut first_error, action, || handler.handle_condense(state));
            }
            Action::CondenseIfFilesTouched => {
                record_first_error(&mut first_error, action, || {
                    handler.handle_condense_if_files_touched(state)
                });
            }
            Action::DiscardIfNoFiles => {
                record_first_error(&mut first_error, action, || {
                    handler.handle_discard_if_no_files(state)
                });
            }
            Action::WarnStaleSession => {
                record_first_error(&mut first_error, action, || {
                    handler.handle_warn_stale_session(state)
                });
            }
        }
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
pub fn mermaid_diagram() -> String {
    let mut out = String::from("stateDiagram-v2\n");
    out.push_str("    state \"IDLE\" as idle\n");
    out.push_str("    state \"ACTIVE\" as active\n");
    out.push_str("    state \"ENDED\" as ended\n");
    out.push('\n');

    for phase in all_phases() {
        for event in all_events() {
            let variants: Vec<(&str, TransitionContext)> =
                if *event == Event::GitCommit && *phase == SessionPhase::Ended {
                    vec![
                        (
                            "[files]",
                            TransitionContext {
                                has_files_touched: true,
                                ..TransitionContext::default()
                            },
                        ),
                        (
                            "[no files]",
                            TransitionContext {
                                has_files_touched: false,
                                ..TransitionContext::default()
                            },
                        ),
                        (
                            "[rebase]",
                            TransitionContext {
                                is_rebase_in_progress: true,
                                ..TransitionContext::default()
                            },
                        ),
                    ]
                } else if *event == Event::GitCommit {
                    vec![
                        ("", TransitionContext::default()),
                        (
                            "[rebase]",
                            TransitionContext {
                                is_rebase_in_progress: true,
                                ..TransitionContext::default()
                            },
                        ),
                    ]
                } else {
                    vec![("", TransitionContext::default())]
                };

            for (variant_label, ctx) in variants {
                let result = transition_with_context(*phase, *event, ctx);
                let mut label = event.to_string();
                if !variant_label.is_empty() {
                    label.push(' ');
                    label.push_str(variant_label);
                }
                if !result.actions.is_empty() {
                    let action_names = result
                        .actions
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    label.push_str(" / ");
                    label.push_str(&action_names);
                }

                out.push_str(&format!(
                    "    {} --> {} : {}\n",
                    phase.as_str(),
                    result.new_phase.as_str(),
                    label
                ));
            }
        }
    }

    out
}

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;

    let mut year = 1970u64;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let months = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &days_in_month in &months {
        let days_in_month = if month == 2 && is_leap(year) {
            29
        } else {
            days_in_month
        };
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }

    let day = remaining + 1;
    (year, month, day, h, mi, s)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
#[path = "phase_tests.rs"]
mod tests;
