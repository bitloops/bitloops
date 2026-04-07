use super::{DepsDirection, DepsKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphqlCompileMode {
    Global,
    Slim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DepsSummaryUnresolvedSelector {
    All,
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DepsSummaryStageSpec {
    pub(super) kind: Option<DepsKind>,
    pub(super) direction: Option<DepsDirection>,
    pub(super) unresolved: Option<DepsSummaryUnresolvedSelector>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RegisteredStageKind<'a> {
    CloneSummary,
    DepsSummary(DepsSummaryStageSpec),
    Tests(&'a super::RegisteredStageCall),
    Coverage,
    TestsSummary,
    Knowledge(&'a super::RegisteredStageCall),
}
