mod lines;
mod message_types;
mod subagents;
mod token_usage;
mod tool_events;

#[cfg(test)]
mod tests;

use crate::host::checkpoints::transcript::types::Line;

pub type TranscriptLine = Line;

pub use lines::{
    extract_last_user_prompt, extract_modified_files, find_checkpoint_uuid, parse_transcript,
    serialize_transcript, truncate_at_uuid,
};
pub use subagents::{extract_all_modified_files, extract_spawned_agent_ids};
pub use token_usage::{
    calculate_token_usage, calculate_token_usage_from_file, calculate_total_token_usage,
};
pub use tool_events::derive_tool_event_observations;
