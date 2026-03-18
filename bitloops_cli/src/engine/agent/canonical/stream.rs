use super::{CanonicalProgressUpdate, CanonicalResultFragment, CanonicalSessionDescriptor};

/// Host-owned streaming event kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CanonicalStreamEventKind {
    #[default]
    Output,
    StreamStart,
    Progress,
    PartialResult,
    DeferredResult,
    StreamEnd,
    Resumed,
}

impl CanonicalStreamEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::StreamStart => "stream_start",
            Self::Progress => "progress",
            Self::PartialResult => "partial_result",
            Self::DeferredResult => "deferred_result",
            Self::StreamEnd => "stream_end",
            Self::Resumed => "resumed",
        }
    }
}

/// Host-owned streaming event payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalStreamEvent {
    pub session: CanonicalSessionDescriptor,
    pub kind: CanonicalStreamEventKind,
    pub sequence: u64,
    pub message: Option<String>,
    pub progress: Option<CanonicalProgressUpdate>,
    pub result: Option<CanonicalResultFragment>,
}

impl CanonicalStreamEvent {
    pub fn new(session: CanonicalSessionDescriptor, kind: CanonicalStreamEventKind) -> Self {
        Self {
            session,
            kind,
            sequence: 0,
            message: None,
            progress: None,
            result: None,
        }
    }

    pub fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = sequence;
        self
    }

    pub fn with_message(mut self, message: impl AsRef<str>) -> Self {
        let message = message.as_ref().trim();
        if !message.is_empty() {
            self.message = Some(message.to_string());
        }
        self
    }

    pub fn with_progress(mut self, progress: CanonicalProgressUpdate) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_result(mut self, result: CanonicalResultFragment) -> Self {
        self.result = Some(result);
        self
    }

    pub fn progress(
        session: CanonicalSessionDescriptor,
        progress: CanonicalProgressUpdate,
    ) -> Self {
        Self::new(session, CanonicalStreamEventKind::Progress).with_progress(progress)
    }

    pub fn partial(session: CanonicalSessionDescriptor, result: CanonicalResultFragment) -> Self {
        Self::new(session, CanonicalStreamEventKind::PartialResult).with_result(result)
    }

    pub fn deferred(session: CanonicalSessionDescriptor, result: CanonicalResultFragment) -> Self {
        Self::new(session, CanonicalStreamEventKind::DeferredResult).with_result(result)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.kind,
            CanonicalStreamEventKind::StreamEnd | CanonicalStreamEventKind::DeferredResult
        ) || self
            .result
            .as_ref()
            .is_some_and(CanonicalResultFragment::is_terminal)
    }
}
