use anyhow::{Result, bail};

use super::args::{
    compile_artefact_args, compile_checkpoint_args, compile_clone_summary_args,
    compile_clones_args, compile_coverage_args, compile_deps_args, compile_deps_summary_args,
    compile_knowledge_args, compile_select_artefacts_args, compile_selection_checkpoint_args,
    compile_selection_clones_args, compile_selection_deps_args, compile_selection_tests_args,
    compile_telemetry_args, compile_tests_args, connection_field, first_arg,
};
use super::document_builder::GraphqlSelection;
use super::document_builder::{GraphqlArgument, GraphqlField};
use super::field_mapping::{
    KNOWLEDGE_STAGE_NAME, SelectableLeaf, clone_result_selections, clone_summary_selections,
    compile_as_of_input, coverage_result_selections, deps_summary_selections, quote_graphql_string,
    scalar_selections_for_leaf, tests_result_selections, tests_summary_result_selections,
};
use super::{GraphqlCompileMode, ParsedDevqlQuery, RegisteredStageCall, RegisteredStageKind};

pub(super) fn compile_terminal_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<GraphqlField> {
    if parsed.select_artefacts.is_some() {
        return compile_select_artefacts_leaf(parsed, registered_stage);
    }

    if parsed.has_checkpoints_stage {
        return compile_checkpoints_leaf(parsed);
    }

    if parsed.has_telemetry_stage {
        return compile_telemetry_leaf(parsed);
    }

    if parsed.has_deps_stage && !parsed.has_artefacts_stage {
        return compile_deps_leaf(parsed);
    }

    if let Some(RegisteredStageKind::Knowledge(stage)) = registered_stage
        && !parsed.has_artefacts_stage
    {
        return compile_knowledge_leaf(parsed, stage);
    }

    if matches!(registered_stage, Some(RegisteredStageKind::CloneSummary)) {
        return compile_clone_summary_leaf(parsed);
    }

    if parsed.has_artefacts_stage {
        return compile_artefacts_leaf(parsed, registered_stage);
    }

    if let Some(RegisteredStageKind::TestsSummary) = registered_stage {
        return Ok(GraphqlField::new(
            "testsSummary",
            Vec::new(),
            tests_summary_result_selections(),
        ));
    }

    bail!("the GraphQL compiler could not determine a queryable leaf stage")
}

fn compile_select_artefacts_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<GraphqlField> {
    let selections = selection_stage_selections(&parsed.select_fields)?;
    let stage_field = if parsed.has_checkpoints_stage {
        GraphqlField::new(
            "checkpoints",
            compile_selection_checkpoint_args(parsed)?,
            selections,
        )
    } else if parsed.has_clones_stage {
        GraphqlField::new(
            "codeMatches",
            compile_selection_clones_args(parsed),
            selections,
        )
    } else if parsed.has_deps_stage {
        GraphqlField::new(
            "dependencies",
            compile_selection_deps_args(parsed),
            selections,
        )
    } else if let Some(RegisteredStageKind::Tests(stage)) = registered_stage {
        GraphqlField::new("tests", compile_selection_tests_args(stage), selections)
    } else {
        bail!("selectArtefacts(...) requires checkpoints(), clones(), deps(), or tests()");
    };

    Ok(GraphqlField::new(
        "selectArtefacts",
        compile_select_artefacts_args(parsed)?,
        vec![stage_field.into()],
    ))
}

pub(super) fn compile_project_stage_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: RegisteredStageKind<'_>,
) -> Result<GraphqlField> {
    match registered_stage {
        RegisteredStageKind::CloneSummary => {
            bail!("summary() is not a project-level registered stage in the GraphQL compiler")
        }
        RegisteredStageKind::DepsSummary(_) => {
            bail!(
                "summary(deps:true, ...) is not a project-level registered stage in the GraphQL compiler"
            )
        }
        RegisteredStageKind::Tests(stage) => Ok(GraphqlField::new(
            "tests",
            compile_tests_args(
                parsed,
                stage,
                true,
                parsed.has_limit_stage.then_some(parsed.limit),
            )?,
            tests_result_selections(),
        )),
        RegisteredStageKind::Coverage => Ok(GraphqlField::new(
            "coverage",
            compile_coverage_args(parsed, true, parsed.has_limit_stage.then_some(parsed.limit))?,
            coverage_result_selections(),
        )),
        RegisteredStageKind::TestsSummary => Ok(GraphqlField::new(
            "testsSummary",
            Vec::new(),
            tests_summary_result_selections(),
        )),
        RegisteredStageKind::Knowledge(_) => {
            bail!("knowledge() is not a project-level registered stage in the GraphQL compiler")
        }
    }
}

fn compile_checkpoints_leaf(parsed: &ParsedDevqlQuery) -> Result<GraphqlField> {
    Ok(connection_field(
        "checkpoints",
        compile_checkpoint_args(parsed)?,
        scalar_selections_for_leaf(SelectableLeaf::Checkpoint, &parsed.select_fields)?,
    ))
}

fn compile_telemetry_leaf(parsed: &ParsedDevqlQuery) -> Result<GraphqlField> {
    Ok(connection_field(
        "telemetry",
        compile_telemetry_args(parsed)?,
        scalar_selections_for_leaf(SelectableLeaf::Telemetry, &parsed.select_fields)?,
    ))
}

fn compile_deps_leaf(parsed: &ParsedDevqlQuery) -> Result<GraphqlField> {
    Ok(connection_field(
        "deps",
        compile_deps_args(parsed, parsed.has_limit_stage.then_some(parsed.limit)),
        scalar_selections_for_leaf(SelectableLeaf::DependencyEdge, &parsed.select_fields)?,
    ))
}

fn compile_knowledge_leaf(
    parsed: &ParsedDevqlQuery,
    stage: &RegisteredStageCall,
) -> Result<GraphqlField> {
    Ok(connection_field(
        KNOWLEDGE_STAGE_NAME,
        compile_knowledge_args(stage, parsed.has_limit_stage.then_some(parsed.limit))?,
        scalar_selections_for_leaf(SelectableLeaf::KnowledgeItem, &parsed.select_fields)?,
    ))
}

fn compile_clone_summary_leaf(parsed: &ParsedDevqlQuery) -> Result<GraphqlField> {
    Ok(GraphqlField::new(
        "cloneSummary",
        compile_clone_summary_args(parsed)?,
        clone_summary_selections(),
    ))
}

fn compile_artefacts_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<GraphqlField> {
    if matches!(registered_stage, Some(RegisteredStageKind::Knowledge(_))) {
        bail!("knowledge() cannot be nested under artefacts() when compiling to GraphQL");
    }

    let mut node_selections = if parsed.has_clones_stage {
        Vec::new()
    } else {
        scalar_selections_for_leaf(SelectableLeaf::Artefact, &parsed.select_fields)?
    };

    if parsed.has_chat_history_stage {
        node_selections.push(
            connection_field(
                "chatHistory",
                first_arg(parsed.has_limit_stage.then_some(parsed.limit)),
                scalar_selections_for_leaf(SelectableLeaf::ChatEntry, &[])?,
            )
            .into(),
        );
    }

    if parsed.has_clones_stage {
        node_selections.push(
            connection_field(
                "clones",
                compile_clones_args(parsed, parsed.has_limit_stage.then_some(parsed.limit)),
                clone_result_selections(parsed.clones.raw)?,
            )
            .into(),
        );
    }

    if let Some(stage) = registered_stage {
        match stage {
            RegisteredStageKind::Tests(stage) => node_selections.push(
                GraphqlField::new(
                    "tests",
                    compile_tests_args(
                        parsed,
                        stage,
                        false,
                        parsed.has_limit_stage.then_some(parsed.limit),
                    )?,
                    tests_result_selections(),
                )
                .into(),
            ),
            RegisteredStageKind::Coverage => node_selections.push(
                GraphqlField::new(
                    "coverage",
                    compile_coverage_args(
                        parsed,
                        false,
                        parsed.has_limit_stage.then_some(parsed.limit),
                    )?,
                    coverage_result_selections(),
                )
                .into(),
            ),
            RegisteredStageKind::CloneSummary => {
                bail!("summary() cannot be nested under artefacts() in the GraphQL compiler")
            }
            RegisteredStageKind::DepsSummary(spec) => node_selections.push(
                GraphqlField::new(
                    "depsSummary",
                    compile_deps_summary_args(spec),
                    deps_summary_selections(),
                )
                .into(),
            ),
            RegisteredStageKind::TestsSummary => {
                bail!("test_harness_tests_summary() cannot be nested under artefacts()")
            }
            RegisteredStageKind::Knowledge(_) => {}
        }
    }

    let outer_first =
        if parsed.has_chat_history_stage || parsed.has_clones_stage || registered_stage.is_some() {
            None
        } else {
            parsed.has_limit_stage.then_some(parsed.limit)
        };

    Ok(connection_field(
        "artefacts",
        compile_artefact_args(parsed, outer_first)?,
        node_selections,
    ))
}

fn selection_stage_selections(select_fields: &[String]) -> Result<Vec<GraphqlSelection>> {
    let fields = if select_fields.is_empty() {
        vec!["summary".to_string()]
    } else {
        select_fields.to_vec()
    };

    let mut selections = Vec::new();
    for field in fields {
        match field.as_str() {
            "summary" => selections.push(GraphqlSelection::scalar("summary")),
            "schema" => selections.push(GraphqlSelection::scalar("schema")),
            other => bail!(
                "selectArtefacts(...) only supports select(summary), select(schema), or selecting both summary and schema; unsupported field `{other}`"
            ),
        }
    }
    Ok(selections)
}

pub(super) fn wrap_in_scopes(
    parsed: &ParsedDevqlQuery,
    terminal_field: GraphqlField,
    mode: GraphqlCompileMode,
) -> GraphqlField {
    let mut current = terminal_field;

    if let Some(file) = parsed.file.as_deref() {
        current = GraphqlField::new(
            "file",
            vec![GraphqlArgument::new("path", quote_graphql_string(file))],
            vec![current.into()],
        );
    } else if let Some(files_path) = parsed.files_path.as_deref() {
        current = GraphqlField::new(
            "files",
            vec![GraphqlArgument::new(
                "path",
                quote_graphql_string(files_path),
            )],
            vec![current.into()],
        );
    }

    if let Some(as_of) = parsed.as_of.as_ref() {
        current = GraphqlField::new(
            "asOf",
            vec![compile_as_of_input(as_of)],
            vec![current.into()],
        );
    }

    if mode == GraphqlCompileMode::Global
        && let Some(project_path) = parsed.project_path.as_deref()
    {
        current = GraphqlField::new(
            "project",
            vec![GraphqlArgument::new(
                "path",
                quote_graphql_string(project_path),
            )],
            vec![current.into()],
        );
    }

    current
}
