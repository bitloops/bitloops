#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphqlCompileMode {
    Global,
    Slim,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RegisteredStageKind<'a> {
    Tests(&'a super::RegisteredStageCall),
    Coverage,
    TestsSummary,
    Knowledge(&'a super::RegisteredStageCall),
}
