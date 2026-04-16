#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SummarySetupSelection {
    Cloud,
    Local,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SummarySetupOutcome {
    InstalledRuntimeOnly,
    Configured { model_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SummarySetupPhase {
    Queued,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WritingProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySetupProgress {
    pub(crate) phase: SummarySetupPhase,
    pub(crate) asset_name: Option<String>,
    pub(crate) bytes_downloaded: u64,
    pub(crate) bytes_total: Option<u64>,
    pub(crate) version: Option<String>,
    pub(crate) message: Option<String>,
}

impl Default for SummarySetupProgress {
    fn default() -> Self {
        Self {
            phase: SummarySetupPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySetupExecutionResult {
    pub(crate) outcome: SummarySetupOutcome,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PreparedSummarySetupAction {
    InstallRuntimeOnly {
        message: String,
    },
    InstallRuntimeOnlyPendingProbe {
        message: String,
    },
    ConfigureLocal {
        model_name: String,
    },
    ConfigureCloud {
        gateway_url_override: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedSummarySetupPlan {
    pub(super) action: PreparedSummarySetupAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OllamaAvailability {
    MissingCli,
    NotRunning,
    Running { models: Vec<String> },
}
