pub mod db_store;
pub(crate) mod interaction_repository;
pub(crate) mod model;
pub(crate) mod projection_ids;
pub(crate) mod query;
pub mod store;
pub(crate) mod tool_events;
pub mod transcript_entry;
pub(crate) mod transcript_fragment;
pub mod transcript_pipeline;
pub mod types;

pub use store::{InteractionEventRepository, InteractionSpool};
pub use transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_derived_tool_use_id, make_entry_id,
};
pub use transcript_pipeline::{
    derive_session_transcript_entries, derive_turn_transcript_entries,
    partition_session_entries_to_turns, read_session_transcript_text,
    synthesize_prompt_fallback_entries,
};
pub use types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionMutation,
    InteractionSession, InteractionTurn,
};
