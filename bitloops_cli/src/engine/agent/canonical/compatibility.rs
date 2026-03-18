/// Host-owned behavioural flags.
///
/// These describe what the host can do; they are not adapter-specific feature
/// toggles and should stay generic across runtimes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCapabilityFlags {
    pub can_install_hooks: bool,
    pub can_resume_sessions: bool,
    pub can_read_transcripts: bool,
    pub can_write_transcripts: bool,
    pub can_report_token_usage: bool,
    pub can_observe_lifecycle_events: bool,
}

impl Default for HostCapabilityFlags {
    fn default() -> Self {
        Self::disabled()
    }
}

impl HostCapabilityFlags {
    pub const fn disabled() -> Self {
        Self {
            can_install_hooks: false,
            can_resume_sessions: false,
            can_read_transcripts: false,
            can_write_transcripts: false,
            can_report_token_usage: false,
            can_observe_lifecycle_events: false,
        }
    }

    pub const fn all_enabled() -> Self {
        Self {
            can_install_hooks: true,
            can_resume_sessions: true,
            can_read_transcripts: true,
            can_write_transcripts: true,
            can_report_token_usage: true,
            can_observe_lifecycle_events: true,
        }
    }
}

/// Explicit versioning for the canonical contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CanonicalContractVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl CanonicalContractVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub const fn current() -> Self {
        Self::new(1, 0, 0)
    }

    pub fn is_compatible_with(self, minimum: Self) -> bool {
        self.major == minimum.major && self >= minimum
    }
}

impl Default for CanonicalContractVersion {
    fn default() -> Self {
        Self::current()
    }
}

/// Compatibility envelope for the richer canonical contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalContractCompatibility {
    pub version: CanonicalContractVersion,
    pub supports_streaming: bool,
    pub supports_progress: bool,
    pub supports_partial_results: bool,
    pub supports_resumable_sessions: bool,
}

impl Default for CanonicalContractCompatibility {
    fn default() -> Self {
        Self::simple()
    }
}

impl CanonicalContractCompatibility {
    pub const fn simple() -> Self {
        Self {
            version: CanonicalContractVersion::current(),
            supports_streaming: false,
            supports_progress: false,
            supports_partial_results: false,
            supports_resumable_sessions: false,
        }
    }

    pub const fn rich() -> Self {
        Self {
            version: CanonicalContractVersion::current(),
            supports_streaming: true,
            supports_progress: true,
            supports_partial_results: true,
            supports_resumable_sessions: true,
        }
    }

    pub fn with_version(mut self, version: CanonicalContractVersion) -> Self {
        self.version = version;
        self
    }

    pub fn with_streaming(mut self, supports_streaming: bool) -> Self {
        self.supports_streaming = supports_streaming;
        self
    }

    pub fn with_progress(mut self, supports_progress: bool) -> Self {
        self.supports_progress = supports_progress;
        self
    }

    pub fn with_partial_results(mut self, supports_partial_results: bool) -> Self {
        self.supports_partial_results = supports_partial_results;
        self
    }

    pub fn with_resumable_sessions(mut self, supports_resumable_sessions: bool) -> Self {
        self.supports_resumable_sessions = supports_resumable_sessions;
        self
    }

    pub fn is_rich(&self) -> bool {
        self.supports_streaming
            || self.supports_progress
            || self.supports_partial_results
            || self.supports_resumable_sessions
    }

    pub fn is_compatible_with(&self, minimum: CanonicalContractVersion) -> bool {
        self.version.is_compatible_with(minimum)
    }
}
