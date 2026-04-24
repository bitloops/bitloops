use async_graphql::{Interface, SimpleObject};

use super::artefact_selection::{CloneExpandHint, DependencyExpandHint};
use super::test_harness::TestHarnessTestsExpandHint;

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ExpandHintParameter {
    pub name: String,
    pub intent: String,
    pub supported_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Interface)]
#[allow(
    clippy::duplicated_attributes,
    reason = "async-graphql Interface derives require one field(... ty = ...) entry per interface field"
)]
#[graphql(
    field(name = "intent", ty = "String"),
    field(name = "template", ty = "String"),
    field(name = "parameters", ty = "Vec<ExpandHintParameter>")
)]
pub enum ExpandHint {
    Tests(TestHarnessTestsExpandHint),
    Clone(CloneExpandHint),
    Dependency(DependencyExpandHint),
}
