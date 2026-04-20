use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::adapters::agents::HookSupport;

use super::agent_api::OpenCodeAgent;
use super::hooks::{BITLOOPS_MARKER, get_plugin_path, render_plugin_template};
use super::skills::{install_repo_skill, repo_skill_path, uninstall_repo_skill};

impl HookSupport for OpenCodeAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        self.install_hooks_at(&repo_root, local_dev, force)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        self.uninstall_hooks_at(&repo_root)
    }

    fn are_hooks_installed(&self) -> bool {
        let repo_root = match crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        }) {
            Ok(repo_root) => repo_root,
            Err(_) => return false,
        };
        self.are_hooks_installed_at(&repo_root)
    }
}

impl OpenCodeAgent {
    pub(crate) fn install_hooks_at(
        &self,
        repo_root: &std::path::Path,
        local_dev: bool,
        force: bool,
    ) -> Result<usize> {
        self.install_hooks_at_with_bitloops_skill(repo_root, local_dev, force, true)
    }

    pub(crate) fn install_hooks_at_with_bitloops_skill(
        &self,
        repo_root: &std::path::Path,
        local_dev: bool,
        force: bool,
        install_bitloops_skill: bool,
    ) -> Result<usize> {
        if install_bitloops_skill {
            install_repo_skill(repo_root)?;
        } else {
            uninstall_repo_skill(repo_root)?;
        }
        let plugin_path = self.plugin_path_at(repo_root);

        if !force
            && plugin_path.exists()
            && let Ok(content) = fs::read_to_string(&plugin_path)
            && Self::ensure_plugin_marker(&content).is_ok()
        {
            return Ok(0);
        }

        let content = self.render_plugin_at(repo_root, local_dev)?;
        if let Some(parent) = plugin_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create plugin directory: {err}"))?;
        }
        fs::write(&plugin_path, content)
            .map_err(|err| anyhow!("failed to write plugin file: {err}"))?;

        Ok(1)
    }

    pub(crate) fn uninstall_hooks_at(&self, repo_root: &std::path::Path) -> Result<()> {
        let plugin_path = self.plugin_path_at(repo_root);
        match fs::remove_file(plugin_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(anyhow!("failed to remove plugin file: {err}")),
        }
        uninstall_repo_skill(repo_root)?;
        Ok(())
    }

    pub(crate) fn are_hooks_installed_at(&self, repo_root: &std::path::Path) -> bool {
        let plugin_path = self.plugin_path_at(repo_root);
        let Ok(content) = fs::read_to_string(plugin_path) else {
            return false;
        };
        let _ = repo_skill_path(repo_root);
        Self::ensure_plugin_marker(&content).is_ok()
    }

    pub(crate) fn plugin_path_at(&self, repo_root: &std::path::Path) -> PathBuf {
        get_plugin_path(repo_root)
    }

    pub fn ensure_plugin_marker(content: &str) -> Result<()> {
        if content.contains(BITLOOPS_MARKER) {
            return Ok(());
        }
        Err(anyhow!("plugin file does not contain Bitloops marker"))
    }

    pub fn render_plugin(&self, local_dev: bool) -> Result<String> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        self.render_plugin_at(&repo_root, local_dev)
    }

    fn render_plugin_at(&self, repo_root: &std::path::Path, local_dev: bool) -> Result<String> {
        render_plugin_template(repo_root, local_dev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn install_hooks_at_installs_plugin_and_repo_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = OpenCodeAgent;

        let count = agent
            .install_hooks_at(dir.path(), false, false)
            .expect("install should succeed");
        assert_eq!(count, 1);

        assert!(
            agent.plugin_path_at(dir.path()).exists(),
            "plugin should be installed"
        );
        assert!(
            repo_skill_path(dir.path()).exists(),
            "repo-local skill should be installed"
        );
    }

    #[test]
    fn uninstall_hooks_at_removes_plugin_and_repo_skill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = OpenCodeAgent;

        agent
            .install_hooks_at(dir.path(), false, false)
            .expect("install should succeed");
        fs::write(dir.path().join(".opencode/skills/keep.txt"), "keep")
            .expect("write sibling file");

        agent
            .uninstall_hooks_at(dir.path())
            .expect("uninstall should succeed");

        assert!(
            !agent.plugin_path_at(dir.path()).exists(),
            "plugin should be removed"
        );
        assert!(
            !repo_skill_path(dir.path()).exists(),
            "repo-local skill should be removed"
        );
        assert!(
            dir.path().join(".opencode/skills").exists(),
            "non-empty parent directory should remain"
        );
    }
}
