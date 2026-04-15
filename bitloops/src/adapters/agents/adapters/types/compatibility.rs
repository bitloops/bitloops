use anyhow::{Result, bail};

pub const HOST_ADAPTER_CONTRACT_VERSION: u16 = 1;
pub const HOST_ADAPTER_RUNTIME_VERSION: u16 = 1;
pub const HOST_PACKAGE_METADATA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentAdapterCapability {
    PresenceDetection,
    ProjectDetection,
    HookInstallation,
    PromptAugmentation,
    SessionIo,
    TranscriptIo,
    TranscriptAnalysis,
    TokenCalculation,
    LifecycleRouting,
}

impl AgentAdapterCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PresenceDetection => "presence_detection",
            Self::ProjectDetection => "project_detection",
            Self::HookInstallation => "hook_installation",
            Self::PromptAugmentation => "prompt_augmentation",
            Self::SessionIo => "session_io",
            Self::TranscriptIo => "transcript_io",
            Self::TranscriptAnalysis => "transcript_analysis",
            Self::TokenCalculation => "token_calculation",
            Self::LifecycleRouting => "lifecycle_routing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentAdapterRuntime {
    LocalCli,
    RemoteRuntime,
}

impl AgentAdapterRuntime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalCli => "local-cli",
            Self::RemoteRuntime => "remote-runtime",
        }
    }
}

const LOCAL_CLI_RUNTIMES: &[AgentAdapterRuntime] = &[AgentAdapterRuntime::LocalCli];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterRuntimeCompatibility {
    pub supported_runtimes: &'static [AgentAdapterRuntime],
}

impl AgentAdapterRuntimeCompatibility {
    pub const fn local_cli() -> Self {
        Self {
            supported_runtimes: LOCAL_CLI_RUNTIMES,
        }
    }

    pub(crate) fn validate(&self, id: &str, scope: &str) -> Result<()> {
        if self.supported_runtimes.is_empty() {
            bail!("{scope} {id} must support at least one runtime");
        }

        if !self
            .supported_runtimes
            .contains(&AgentAdapterRuntime::LocalCli)
        {
            bail!(
                "{scope} {id} is incompatible with host runtime {}",
                AgentAdapterRuntime::LocalCli.as_str()
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterCompatibility {
    pub contract_version: u16,
    pub min_host_version: u16,
    pub max_host_version: u16,
}

impl AgentAdapterCompatibility {
    pub const fn phase1() -> Self {
        Self {
            contract_version: HOST_ADAPTER_CONTRACT_VERSION,
            min_host_version: HOST_ADAPTER_RUNTIME_VERSION,
            max_host_version: HOST_ADAPTER_RUNTIME_VERSION,
        }
    }

    pub(crate) fn validate(&self, id: &str, scope: &str) -> Result<()> {
        if self.contract_version != HOST_ADAPTER_CONTRACT_VERSION {
            bail!(
                "{scope} {id} has unsupported contract version {} (expected {})",
                self.contract_version,
                HOST_ADAPTER_CONTRACT_VERSION
            );
        }
        if HOST_ADAPTER_RUNTIME_VERSION < self.min_host_version
            || HOST_ADAPTER_RUNTIME_VERSION > self.max_host_version
        {
            bail!(
                "{scope} {id} is incompatible with host runtime version {} (supported {}-{})",
                HOST_ADAPTER_RUNTIME_VERSION,
                self.min_host_version,
                self.max_host_version
            );
        }
        Ok(())
    }
}
