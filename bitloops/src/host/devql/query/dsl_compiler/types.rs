#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphqlCompileMode {
    Global,
    Slim,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RegisteredStageKind<'a> {
    CloneSummary,
    Tests(&'a super::RegisteredStageCall),
    Coverage,
    TestsSummary,
    Knowledge(&'a super::RegisteredStageCall),
}
