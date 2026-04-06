//! Checkpoint session hook lifecycle: parsing hook stdin, dispatching events, and persisting
//! session / interaction state across agent families.

mod adapter;
pub(crate) mod canonical;
mod capture;
mod dispatch;
mod git_workspace;
mod handlers_session;
mod handlers_tail;
mod interaction;
mod time_and_ids;
mod transcript;
mod turn_end;
mod types;

pub mod adapters;

pub use adapter::LifecycleAgentAdapter;
pub use capture::capture_pre_prompt_state;
pub use dispatch::dispatch_lifecycle_event;
pub use handlers_session::{handle_lifecycle_session_start, handle_lifecycle_turn_start};
pub use handlers_tail::{
    handle_lifecycle_compaction, handle_lifecycle_session_end, handle_lifecycle_subagent_end,
    handle_lifecycle_subagent_start,
};
pub use transcript::{create_context_file, read_and_parse_hook_input, resolve_transcript_offset};
pub use turn_end::handle_lifecycle_turn_end;
pub use types::{
    LifecycleEvent, LifecycleEventType, PrePromptState, SessionIdPolicy, UNKNOWN_SESSION_ID,
    apply_session_id_policy,
};

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod orchestration_tests;
