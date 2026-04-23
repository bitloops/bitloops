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
#[graphql(field(name = "intent", ty = "String"))]
#[graphql(field(name = "template", ty = "String"))]
#[graphql(field(name = "parameters", ty = "Vec<ExpandHintParameter>"))]
pub enum ExpandHint {
    TestHarnessTestsExpandHint(TestHarnessTestsExpandHint),
    CloneExpandHint(CloneExpandHint),
    DependencyExpandHint(DependencyExpandHint),
}
