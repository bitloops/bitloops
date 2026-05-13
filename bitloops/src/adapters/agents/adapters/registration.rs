use anyhow::Result;
use std::path::Path;

use super::super::Agent;
use super::types::AgentAdapterDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentHookInstallOptions {
    pub install_bitloops_skill: bool,
}

impl Default for AgentHookInstallOptions {
    fn default() -> Self {
        Self {
            install_bitloops_skill: true,
        }
    }
}

#[derive(Debug)]
pub struct AgentAdapterRegistration {
    descriptor: AgentAdapterDescriptor,
    create_agent: fn() -> Box<dyn Agent + Send + Sync>,
    detect_project_presence: fn(&Path) -> bool,
    hooks_installed: fn(&Path) -> bool,
    install_hooks: fn(&Path, bool, bool, AgentHookInstallOptions) -> Result<usize>,
    uninstall_hooks: fn(&Path) -> Result<()>,
    prompt_surface_installed: fn(&Path) -> bool,
    install_prompt_surface: fn(&Path) -> Result<bool>,
    uninstall_prompt_surface: fn(&Path) -> Result<()>,
    format_resume_command: fn(&str) -> String,
}

impl AgentAdapterRegistration {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        descriptor: AgentAdapterDescriptor,
        create_agent: fn() -> Box<dyn Agent + Send + Sync>,
        detect_project_presence: fn(&Path) -> bool,
        hooks_installed: fn(&Path) -> bool,
        install_hooks: fn(&Path, bool, bool, AgentHookInstallOptions) -> Result<usize>,
        uninstall_hooks: fn(&Path) -> Result<()>,
        prompt_surface_installed: fn(&Path) -> bool,
        install_prompt_surface: fn(&Path) -> Result<bool>,
        uninstall_prompt_surface: fn(&Path) -> Result<()>,
        format_resume_command: fn(&str) -> String,
    ) -> Self {
        Self {
            descriptor,
            create_agent,
            detect_project_presence,
            hooks_installed,
            install_hooks,
            uninstall_hooks,
            prompt_surface_installed,
            install_prompt_surface,
            uninstall_prompt_surface,
            format_resume_command,
        }
    }

    pub fn descriptor(&self) -> &AgentAdapterDescriptor {
        &self.descriptor
    }

    pub fn create_agent(&self) -> Box<dyn Agent + Send + Sync> {
        (self.create_agent)()
    }

    pub fn is_project_detected(&self, repo_root: &Path) -> bool {
        (self.detect_project_presence)(repo_root)
    }

    pub fn are_hooks_installed(&self, repo_root: &Path) -> bool {
        (self.hooks_installed)(repo_root)
    }

    pub fn install_hooks(
        &self,
        repo_root: &Path,
        local_dev: bool,
        force: bool,
        options: AgentHookInstallOptions,
    ) -> Result<usize> {
        (self.install_hooks)(repo_root, local_dev, force, options)
    }

    pub fn uninstall_hooks(&self, repo_root: &Path) -> Result<()> {
        (self.uninstall_hooks)(repo_root)
    }

    pub fn is_prompt_surface_installed(&self, repo_root: &Path) -> bool {
        (self.prompt_surface_installed)(repo_root)
    }

    pub fn install_prompt_surface(&self, repo_root: &Path) -> Result<bool> {
        (self.install_prompt_surface)(repo_root)
    }

    pub fn uninstall_prompt_surface(&self, repo_root: &Path) -> Result<()> {
        (self.uninstall_prompt_surface)(repo_root)
    }

    pub fn format_resume_command(&self, session_id: &str) -> String {
        (self.format_resume_command)(session_id)
    }
}
