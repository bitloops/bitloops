use std::fs;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::adapters::agents::HookSupport;

use super::agent_api::OpenCodeAgent;
use super::hooks::{BITLOOPS_MARKER, get_plugin_path, render_plugin_template};

impl HookSupport for OpenCodeAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        let plugin_path = self.plugin_path()?;

        if !force
            && plugin_path.exists()
            && let Ok(content) = fs::read_to_string(&plugin_path)
            && Self::ensure_plugin_marker(&content).is_ok()
        {
            return Ok(0);
        }

        let content = self.render_plugin(local_dev)?;
        if let Some(parent) = plugin_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create plugin directory: {err}"))?;
        }
        fs::write(&plugin_path, content)
            .map_err(|err| anyhow!("failed to write plugin file: {err}"))?;

        Ok(1)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        let plugin_path = self.plugin_path()?;
        match fs::remove_file(plugin_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(anyhow!("failed to remove plugin file: {err}")),
        }
    }

    fn are_hooks_installed(&self) -> bool {
        let Ok(plugin_path) = self.plugin_path() else {
            return false;
        };
        let Ok(content) = fs::read_to_string(plugin_path) else {
            return false;
        };
        Self::ensure_plugin_marker(&content).is_ok()
    }
}

impl OpenCodeAgent {
    pub fn plugin_path(&self) -> Result<PathBuf> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        Ok(get_plugin_path(&repo_root))
    }

    pub fn ensure_plugin_marker(content: &str) -> Result<()> {
        if content.contains(BITLOOPS_MARKER) {
            return Ok(());
        }
        Err(anyhow!("plugin file does not contain Bitloops marker"))
    }

    pub fn render_plugin(&self, local_dev: bool) -> Result<String> {
        render_plugin_template(local_dev)
    }
}
