use anyhow::Result;
use std::path::Path;

use super::super::Agent;
use super::types::AgentAdapterDescriptor;
use crate::host::hooks::augmentation::builder::HookAugmentation;

pub type PromptAugmentationRenderer = fn(&str, &HookAugmentation) -> Option<String>;

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
    format_resume_command: fn(&str) -> String,
    render_prompt_augmentation: Option<PromptAugmentationRenderer>,
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
        format_resume_command: fn(&str) -> String,
        render_prompt_augmentation: Option<PromptAugmentationRenderer>,
    ) -> Self {
        Self {
            descriptor,
            create_agent,
            detect_project_presence,
            hooks_installed,
            install_hooks,
            uninstall_hooks,
            format_resume_command,
            render_prompt_augmentation,
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

    pub fn format_resume_command(&self, session_id: &str) -> String {
        (self.format_resume_command)(session_id)
    }

    pub fn render_prompt_augmentation(
        &self,
        hook_name: &str,
        augmentation: &HookAugmentation,
    ) -> Option<String> {
        self.render_prompt_augmentation
            .and_then(|render| render(hook_name, augmentation))
    }
}
