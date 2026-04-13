#[path = "classification/classifier.rs"]
mod classifier;
#[path = "classification/context.rs"]
mod context;
#[path = "classification/path_rules.rs"]
mod path_rules;
#[path = "classification/patterns.rs"]
mod patterns;
#[path = "classification/repo_view.rs"]
mod repo_view;
#[cfg(test)]
#[path = "classification/tests.rs"]
mod tests;
#[path = "classification/types.rs"]
mod types;

pub(crate) use self::classifier::ProjectAwareClassifier;
pub(crate) use self::types::{
    AnalysisMode, FileRole, ProjectContext, ResolvedFileClassification, TextIndexMode,
};
