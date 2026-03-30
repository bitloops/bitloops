use anyhow::Result;

use super::*;

#[path = "dsl_compiler/args.rs"]
mod args;
#[path = "dsl_compiler/document_builder.rs"]
mod document_builder;
#[path = "dsl_compiler/field_mapping.rs"]
mod field_mapping;
#[path = "dsl_compiler/leaves.rs"]
mod leaves;
#[path = "dsl_compiler/types.rs"]
mod types;
#[path = "dsl_compiler/validation.rs"]
mod validation;

#[cfg(test)]
#[path = "dsl_compiler/tests.rs"]
mod tests;

use self::document_builder::{GraphqlArgument, GraphqlDocumentBuilder, GraphqlField};
use self::field_mapping::quote_graphql_string;
pub(crate) use self::types::GraphqlCompileMode;
use self::types::RegisteredStageKind;
use self::validation::{resolve_registered_stage, should_compile_project_stage};

pub fn compile_devql_query_to_graphql(query: &str) -> Result<String> {
    let parsed = parse_devql_query(query)?;
    compile_devql_to_graphql_with_mode(&parsed, GraphqlCompileMode::Global)
}

#[cfg(test)]
pub(crate) fn compile_devql_to_graphql(parsed: &ParsedDevqlQuery) -> Result<String> {
    compile_devql_to_graphql_with_mode(parsed, GraphqlCompileMode::Global)
}

pub(crate) fn compile_devql_to_graphql_with_mode(
    parsed: &ParsedDevqlQuery,
    mode: GraphqlCompileMode,
) -> Result<String> {
    let registered_stage = resolve_registered_stage(parsed)?;
    validation::validate_graphql_compiler_support(parsed, registered_stage, mode)?;

    let terminal_field = if should_compile_project_stage(parsed, registered_stage) {
        leaves::compile_project_stage_leaf(parsed, registered_stage.expect("checked above"))?
    } else {
        leaves::compile_terminal_leaf(parsed, registered_stage)?
    };

    let scoped_field = leaves::wrap_in_scopes(parsed, terminal_field, mode);
    Ok(match mode {
        GraphqlCompileMode::Global => {
            let repo_name = parsed.repo.as_deref().unwrap_or("default");
            GraphqlDocumentBuilder::new(vec![GraphqlField::new(
                "repo",
                vec![GraphqlArgument::new(
                    "name",
                    quote_graphql_string(repo_name),
                )],
                vec![scoped_field.into()],
            )])
            .build()
        }
        GraphqlCompileMode::Slim => GraphqlDocumentBuilder::new(vec![scoped_field]).build(),
    })
}
