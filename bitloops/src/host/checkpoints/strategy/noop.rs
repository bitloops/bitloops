//! No-op strategy — does nothing. Used in tests and as a placeholder.

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

    fn post_commit(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn noop_strategy_accepts_all_strategy_operations() {
        let strategy = NoOpStrategy;

        assert_eq!(strategy.name(), "noop");
        strategy
            .save_step(&StepContext::default())
            .expect("save_step should be a no-op");
        strategy
            .save_task_step(&TaskStepContext::default())
            .expect("save_task_step should be a no-op");
        strategy
            .prepare_commit_msg(Path::new("/tmp/ignored"), Some("message"))
            .expect("prepare_commit_msg should be a no-op");
        strategy
            .commit_msg(Path::new("/tmp/ignored"))
            .expect("commit_msg should be a no-op");
        strategy
            .post_commit()
            .expect("post_commit should be a no-op");
        strategy
            .pre_push("origin")
            .expect("pre_push should be a no-op");
        strategy
            .post_merge(false)
            .expect("post_merge should be a no-op");
        strategy
            .post_checkout("old-head", "new-head", true)
            .expect("post_checkout should be a no-op");
        strategy
            .reference_transaction("committed", &[])
            .expect("reference_transaction should be a no-op");
    }
}
