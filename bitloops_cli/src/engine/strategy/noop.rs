//! No-op strategy — does nothing. Used in tests and as a placeholder.

use std::path::Path;

use anyhow::Result;

use super::{StepContext, Strategy, TaskStepContext};

/// A strategy that accepts all calls and does nothing.
pub struct NoOpStrategy;

impl Strategy for NoOpStrategy {
    fn name(&self) -> &str {
        "noop"
    }

    fn save_step(&self, _ctx: &StepContext) -> Result<()> {
        Ok(())
    }

    fn save_task_step(&self, _ctx: &TaskStepContext) -> Result<()> {
        Ok(())
    }

    fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
        Ok(())
    }

    fn post_commit(&self) -> Result<()> {
        Ok(())
    }

    fn pre_push(&self, _remote: &str) -> Result<()> {
        Ok(())
    }
}
