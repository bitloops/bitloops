use super::{DepsDirection, DepsKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphqlCompileMode {
    Global,
    Slim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DepsSummaryStageSpec {
    pub(super) kind: Option<DepsKind>,
    pub(super) direction: Option<DepsDirection>,
    pub(super) unresolved: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RegisteredStageKind<'a> {
    CloneSummary,
    DepsSummary(DepsSummaryStageSpec),
    Tests(&'a super::RegisteredStageCall),
    Coverage,
    TestsSummary,
    Knowledge(&'a super::RegisteredStageCall),
    SelectionOverview,
    HttpSearch(&'a super::RegisteredStageCall),
    HttpContext(&'a super::RegisteredStageCall),
    HttpHeaderProducers(&'a super::RegisteredStageCall),
    HttpLifecycleBoundaries(&'a super::RegisteredStageCall),
    HttpLossyTransforms(&'a super::RegisteredStageCall),
    HttpPatchImpact(&'a super::RegisteredStageCall),
}
