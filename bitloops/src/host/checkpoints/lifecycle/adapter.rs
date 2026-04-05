use anyhow::Result;
use std::io::Read;

use super::types::LifecycleEvent;

pub trait LifecycleAgentAdapter: Send + Sync {
    fn agent_name(&self) -> &'static str;
    fn parse_hook_event(
        &self,
        _hook_name: &str,
        _stdin: &mut dyn Read,
    ) -> Result<Option<LifecycleEvent>>;
    fn hook_names(&self) -> Vec<&'static str>;
    fn format_resume_command(&self, _session_id: &str) -> String;

    /// When present, used by handle_lifecycle_turn_end to extract prompts, summary, and modified files.
    fn as_transcript_analyzer(&self) -> Option<&dyn crate::adapters::agents::TranscriptAnalyzer> {
        None
    }

    /// When present, used by handle_lifecycle_turn_end to include token usage in the saved step.
    fn as_token_calculator(&self) -> Option<&dyn crate::adapters::agents::TokenCalculator> {
        None
    }
}
