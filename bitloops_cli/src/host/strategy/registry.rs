//! Strategy registry.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};

use super::Strategy;
use super::auto_commit::AutoCommitStrategy;
use super::manual_commit::ManualCommitStrategy;

pub const STRATEGY_NAME_MANUAL_COMMIT: &str = "manual-commit";
pub const STRATEGY_NAME_AUTO_COMMIT: &str = "auto-commit";

pub type Factory = fn(&Path) -> Box<dyn Strategy>;

fn manual_factory(repo_root: &Path) -> Box<dyn Strategy> {
    Box::new(ManualCommitStrategy::new(repo_root))
}

fn auto_factory(repo_root: &Path) -> Box<dyn Strategy> {
    Box::new(AutoCommitStrategy::new(repo_root))
}

/// Immutable registry of known strategies, constructed once at startup.
pub struct StrategyRegistry {
    factories: HashMap<String, Factory>,
}

impl StrategyRegistry {
    /// Build the default registry containing all built-in strategies.
    pub fn builtin() -> Self {
        let mut factories = HashMap::new();
        factories.insert(
            STRATEGY_NAME_MANUAL_COMMIT.to_string(),
            manual_factory as Factory,
        );
        factories.insert(
            STRATEGY_NAME_AUTO_COMMIT.to_string(),
            auto_factory as Factory,
        );
        Self { factories }
    }

    pub fn get(&self, name: &str, repo_root: &Path) -> Result<Box<dyn Strategy>> {
        match self.factories.get(name).copied() {
            Some(f) => Ok(f(repo_root)),
            None => Err(anyhow!(
                "unknown strategy: {name} (available: {:?})",
                self.list()
            )),
        }
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.factories.keys().cloned().collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn manual_commit_strategy_registration() {
        let dir = tempdir().expect("tempdir");
        let registry = StrategyRegistry::builtin();

        let strategy = registry
            .get(STRATEGY_NAME_MANUAL_COMMIT, dir.path())
            .expect("Get(manual-commit) should return a registered strategy");

        assert_eq!(
            strategy.name(),
            STRATEGY_NAME_MANUAL_COMMIT,
            "Name() should match the manual-commit registry constant"
        );
    }

    #[test]
    fn auto_commit_strategy_registration() {
        let dir = tempdir().expect("tempdir");
        let registry = StrategyRegistry::builtin();

        let strategy = registry
            .get(STRATEGY_NAME_AUTO_COMMIT, dir.path())
            .expect("Get(auto-commit) should return a registered strategy");

        assert_eq!(
            strategy.name(),
            STRATEGY_NAME_AUTO_COMMIT,
            "Name() should match the auto-commit registry constant"
        );
    }
}
