use super::CanonicalSessionDescriptor;

/// Host-owned result disposition for partial and deferred flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CanonicalResultState {
    #[default]
    Final,
    Partial,
    Deferred,
}

impl CanonicalResultState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Partial => "partial",
            Self::Deferred => "deferred",
        }
    }
}

/// Host-owned resumable session state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CanonicalResumableSessionState {
    #[default]
    Active,
    Suspended,
    Resumable,
    Resumed,
    Completed,
}

impl CanonicalResumableSessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Resumable => "resumable",
            Self::Resumed => "resumed",
            Self::Completed => "completed",
        }
    }
}

/// Host-owned resumable session descriptor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalResumableSession {
    pub session: CanonicalSessionDescriptor,
    pub state: CanonicalResumableSessionState,
    pub checkpoint: Option<String>,
    pub resume_token: Option<String>,
    pub last_sequence: Option<u64>,
    pub note: Option<String>,
}

impl CanonicalResumableSession {
    pub fn new(session: CanonicalSessionDescriptor) -> Self {
        Self {
            session,
            state: CanonicalResumableSessionState::Active,
            checkpoint: None,
            resume_token: None,
            last_sequence: None,
            note: None,
        }
    }

    pub fn with_checkpoint(mut self, checkpoint: impl AsRef<str>) -> Self {
        let checkpoint = checkpoint.as_ref().trim();
        if !checkpoint.is_empty() {
            self.checkpoint = Some(checkpoint.to_string());
        }
        self
    }

    pub fn with_resume_token(mut self, resume_token: impl AsRef<str>) -> Self {
        let resume_token = resume_token.as_ref().trim();
        if !resume_token.is_empty() {
            self.resume_token = Some(resume_token.to_string());
        }
        self
    }

    pub fn with_last_sequence(mut self, last_sequence: u64) -> Self {
        self.last_sequence = Some(last_sequence);
        self
    }

    pub fn with_note(mut self, note: impl AsRef<str>) -> Self {
        let note = note.as_ref().trim();
        if !note.is_empty() {
            self.note = Some(note.to_string());
        }
        self
    }

    pub fn mark_suspended(mut self) -> Self {
        self.state = CanonicalResumableSessionState::Suspended;
        self
    }

    pub fn mark_resumable(mut self) -> Self {
        self.state = CanonicalResumableSessionState::Resumable;
        self
    }

    pub fn mark_resumed(mut self) -> Self {
        self.state = CanonicalResumableSessionState::Resumed;
        self
    }

    pub fn mark_completed(mut self) -> Self {
        self.state = CanonicalResumableSessionState::Completed;
        self
    }

    pub fn can_resume(&self) -> bool {
        matches!(
            self.state,
            CanonicalResumableSessionState::Suspended | CanonicalResumableSessionState::Resumable
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.state, CanonicalResumableSessionState::Completed)
    }
}

/// Host-owned partial, deferred, or final result fragment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalResultFragment {
    pub state: CanonicalResultState,
    pub content: Option<String>,
    pub reason: Option<String>,
    pub resumable_session: Option<CanonicalResumableSession>,
}

impl CanonicalResultFragment {
    pub fn final_output(output: impl AsRef<str>) -> Self {
        let output = output.as_ref().trim();
        Self {
            state: CanonicalResultState::Final,
            content: (!output.is_empty()).then(|| output.to_string()),
            reason: None,
            resumable_session: None,
        }
    }

    pub fn partial(content: impl AsRef<str>) -> Self {
        let content = content.as_ref().trim();
        Self {
            state: CanonicalResultState::Partial,
            content: (!content.is_empty()).then(|| content.to_string()),
            reason: None,
            resumable_session: None,
        }
    }

    pub fn deferred(reason: impl AsRef<str>) -> Self {
        let reason = reason.as_ref().trim();
        Self {
            state: CanonicalResultState::Deferred,
            content: None,
            reason: (!reason.is_empty()).then(|| reason.to_string()),
            resumable_session: None,
        }
    }

    pub fn with_reason(mut self, reason: impl AsRef<str>) -> Self {
        let reason = reason.as_ref().trim();
        if !reason.is_empty() {
            self.reason = Some(reason.to_string());
        }
        self
    }

    pub fn with_resumable_session(mut self, resumable_session: CanonicalResumableSession) -> Self {
        self.resumable_session = Some(resumable_session);
        self
    }

    pub fn is_final(&self) -> bool {
        matches!(self.state, CanonicalResultState::Final)
    }

    pub fn is_partial(&self) -> bool {
        matches!(self.state, CanonicalResultState::Partial)
    }

    pub fn is_deferred(&self) -> bool {
        matches!(self.state, CanonicalResultState::Deferred)
    }

    pub fn is_terminal(&self) -> bool {
        self.is_final() || self.is_deferred()
    }
}
