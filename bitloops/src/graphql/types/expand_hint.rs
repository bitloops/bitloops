use async_graphql::{Interface, Union};

use super::artefact_selection::{
    CloneExpandHint, CloneExpandHintParameters, DependencyExpandHint,
    DependencyExpandHintParameters,
};
use super::test_harness::TestHarnessTestsExpandHint;

#[derive(Debug, Clone, PartialEq, Eq, async_graphql::SimpleObject)]
pub struct ExpandHintParameter {
    pub intent: String,
    pub supported_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Union)]
pub enum ExpandHintParameters {
    CloneExpandHintParameters(CloneExpandHintParameters),
    DependencyExpandHintParameters(DependencyExpandHintParameters),
}

impl From<&CloneExpandHintParameters> for ExpandHintParameters {
    fn from(value: &CloneExpandHintParameters) -> Self {
        Self::CloneExpandHintParameters(value.clone())
    }
}

impl From<&DependencyExpandHintParameters> for ExpandHintParameters {
    fn from(value: &DependencyExpandHintParameters) -> Self {
        Self::DependencyExpandHintParameters(value.clone())
    }
}

impl From<&CloneExpandHintParameters> for Option<ExpandHintParameters> {
    fn from(value: &CloneExpandHintParameters) -> Self {
        Some(value.into())
    }
}

impl From<&DependencyExpandHintParameters> for Option<ExpandHintParameters> {
    fn from(value: &DependencyExpandHintParameters) -> Self {
        Some(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Interface)]
#[graphql(field(name = "intent", ty = "String"))]
#[graphql(field(name = "template", ty = "String"))]
#[graphql(field(name = "parameters", ty = "Option<ExpandHintParameters>"))]
pub enum ExpandHint {
    TestHarnessTestsExpandHint(TestHarnessTestsExpandHint),
    CloneExpandHint(CloneExpandHint),
    DependencyExpandHint(DependencyExpandHint),
}
