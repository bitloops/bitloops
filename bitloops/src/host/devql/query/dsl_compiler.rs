use anyhow::{Result, bail};

use super::*;

#[path = "dsl_compiler/document_builder.rs"]
mod document_builder;
#[path = "dsl_compiler/field_mapping.rs"]
mod field_mapping;

use self::document_builder::{
    GraphqlArgument, GraphqlDocumentBuilder, GraphqlField, GraphqlSelection,
};
use self::field_mapping::{
    KNOWLEDGE_STAGE_NAME, SelectableLeaf, classify_registered_stage, compile_as_of_input,
    compile_datetime_literal, compile_stage_json_literal, coverage_result_selections, enum_literal,
    quote_graphql_string, scalar_selections_for_leaf, tests_result_selections,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum RegisteredStageKind<'a> {
    Tests(&'a RegisteredStageCall),
    Coverage,
    Knowledge(&'a RegisteredStageCall),
    Extension(&'a RegisteredStageCall),
}

pub fn compile_devql_query_to_graphql(query: &str) -> Result<String> {
    let parsed = parse_devql_query(query)?;
    compile_devql_to_graphql(&parsed)
}

pub(crate) fn compile_devql_to_graphql(parsed: &ParsedDevqlQuery) -> Result<String> {
    let registered_stage = resolve_registered_stage(parsed)?;
    validate_graphql_compiler_support(parsed, registered_stage)?;

    let terminal_field = if should_compile_project_stage(parsed, registered_stage) {
        compile_project_stage_leaf(parsed, registered_stage.expect("checked above"))?
    } else {
        compile_terminal_leaf(parsed, registered_stage)?
    };

    let scoped_field = wrap_in_scopes(parsed, terminal_field);
    let repo_name = parsed.repo.as_deref().unwrap_or("default");

    Ok(GraphqlDocumentBuilder::new(vec![GraphqlField::new(
        "repo",
        vec![GraphqlArgument::new(
            "name",
            quote_graphql_string(repo_name),
        )],
        vec![scoped_field.into()],
    )])
    .build())
}

fn resolve_registered_stage(parsed: &ParsedDevqlQuery) -> Result<Option<RegisteredStageKind<'_>>> {
    if parsed.registered_stages.len() > 1 {
        bail!(
            "the GraphQL compiler does not yet support multiple registered capability-pack stages in one query"
        );
    }

    Ok(parsed
        .registered_stages
        .first()
        .map(classify_registered_stage))
}

fn validate_graphql_compiler_support(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<()> {
    if parsed.file.is_some() && parsed.files_path.is_some() {
        bail!("file() cannot be combined with files() in one query");
    }

    if (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
        && (parsed.file.is_some() || parsed.files_path.is_some() || parsed.has_artefacts_stage)
    {
        bail!(
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query"
        );
    }

    if parsed.has_chat_history_stage && !parsed.has_artefacts_stage {
        bail!("chatHistory() requires an artefacts() stage in the query");
    }

    if parsed.has_clones_stage && !parsed.has_artefacts_stage {
        bail!("clones() requires an artefacts() stage in the query");
    }

    if parsed.has_deps_stage && parsed.has_chat_history_stage {
        bail!("deps() cannot be combined with chatHistory() stage");
    }

    if parsed.has_clones_stage && parsed.has_deps_stage {
        bail!("clones() cannot be combined with deps() stage");
    }

    if parsed.has_chat_history_stage && (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
    {
        bail!("chatHistory() cannot be combined with checkpoints()/telemetry() stages");
    }

    if parsed.has_clones_stage && parsed.has_chat_history_stage {
        bail!("clones() cannot be combined with chatHistory() stage");
    }

    if parsed.has_clones_stage && parsed.as_of.is_some() {
        bail!("clones() does not yet support asOf(...) queries");
    }

    let has_tests_stage = matches!(registered_stage, Some(RegisteredStageKind::Tests(_)));
    let has_coverage_stage = matches!(registered_stage, Some(RegisteredStageKind::Coverage));

    if has_tests_stage && !parsed.has_artefacts_stage {
        bail!("tests() requires an artefacts() stage in the query");
    }

    if has_tests_stage && parsed.has_deps_stage {
        bail!("tests() cannot be combined with deps() stage");
    }

    if has_tests_stage && parsed.has_clones_stage {
        bail!("tests() cannot be combined with clones() stage");
    }

    if has_tests_stage && parsed.has_chat_history_stage {
        bail!("tests() cannot be combined with chatHistory() stage");
    }

    if has_coverage_stage && has_tests_stage {
        bail!("coverage() cannot be combined with tests() stage");
    }

    if has_coverage_stage && !parsed.has_artefacts_stage {
        bail!("coverage() requires an artefacts() stage in the query");
    }

    if has_coverage_stage && parsed.has_deps_stage {
        bail!("coverage() cannot be combined with deps() stage");
    }

    if has_coverage_stage && parsed.has_clones_stage {
        bail!("coverage() cannot be combined with clones() stage");
    }

    if has_coverage_stage && parsed.has_chat_history_stage {
        bail!("coverage() cannot be combined with chatHistory() stage");
    }

    if parsed.has_deps_stage && parsed.has_artefacts_stage {
        bail!("deps() after artefacts() is not yet supported by the GraphQL compiler");
    }

    if parsed.has_telemetry_stage && parsed.project_path.is_some() {
        bail!("telemetry() does not support project() scoping in the GraphQL schema yet");
    }

    if parsed.has_telemetry_stage && parsed.as_of.is_some() {
        bail!("telemetry() does not support asOf(...) in the GraphQL schema yet");
    }

    if parsed.has_checkpoints_stage && parsed.as_of.is_some() {
        bail!("checkpoints() does not support asOf(...) in the GraphQL schema yet");
    }

    if parsed.has_deps_stage
        && !parsed.has_artefacts_stage
        && parsed.project_path.is_none()
        && parsed.file.is_none()
        && parsed.files_path.is_none()
    {
        bail!("deps() requires project(), file(), or files() when compiling to GraphQL");
    }

    if parsed.has_deps_stage
        && parsed.as_of.is_some()
        && parsed.file.is_none()
        && parsed.files_path.is_none()
    {
        bail!(
            "deps() with asOf(...) is only supported when further scoped through file() or files()"
        );
    }

    if matches!(registered_stage, Some(RegisteredStageKind::Knowledge(_)))
        && (parsed.has_artefacts_stage
            || parsed.has_checkpoints_stage
            || parsed.has_telemetry_stage
            || parsed.has_deps_stage
            || parsed.has_clones_stage
            || parsed.has_chat_history_stage)
    {
        bail!(
            "knowledge() cannot currently be combined with other query stages in the GraphQL compiler"
        );
    }

    if matches!(registered_stage, Some(RegisteredStageKind::Knowledge(_)))
        && (parsed.as_of.is_some() || parsed.file.is_some() || parsed.files_path.is_some())
    {
        bail!(
            "knowledge() does not support asOf(...), file(), or files() scopes in the GraphQL schema yet"
        );
    }

    if matches!(registered_stage, Some(RegisteredStageKind::Extension(_)))
        && !parsed.has_artefacts_stage
        && parsed.project_path.is_none()
    {
        bail!(
            "registered capability-pack stages require artefacts() or project() when compiling to GraphQL"
        );
    }

    if matches!(registered_stage, Some(RegisteredStageKind::Extension(_)))
        && !parsed.has_artefacts_stage
        && (parsed.as_of.is_some() || parsed.file.is_some() || parsed.files_path.is_some())
    {
        bail!(
            "project-level capability-pack stages do not support asOf(...), file(), or files() scopes in the GraphQL compiler"
        );
    }

    if parsed.has_artefacts_stage
        || parsed.has_checkpoints_stage
        || parsed.has_telemetry_stage
        || parsed.has_deps_stage
        || matches!(
            registered_stage,
            Some(RegisteredStageKind::Knowledge(_)) | Some(RegisteredStageKind::Extension(_))
        )
    {
        return Ok(());
    }

    bail!("the GraphQL compiler could not determine a queryable leaf stage")
}

fn should_compile_project_stage(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> bool {
    let Some(registered_stage) = registered_stage else {
        return false;
    };

    if parsed.as_of.is_some()
        || parsed.file.is_some()
        || parsed.files_path.is_some()
        || parsed.has_chat_history_stage
        || parsed.has_clones_stage
        || parsed.has_deps_stage
        || !parsed.select_fields.is_empty()
    {
        return false;
    }

    match registered_stage {
        RegisteredStageKind::Tests(_)
        | RegisteredStageKind::Coverage
        | RegisteredStageKind::Extension(_) => parsed.project_path.is_some(),
        RegisteredStageKind::Knowledge(_) => false,
    }
}

fn compile_terminal_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<GraphqlField> {
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

    if parsed.has_artefacts_stage {
        return compile_artefacts_leaf(parsed, registered_stage);
    }

    if let Some(RegisteredStageKind::Extension(stage)) = registered_stage {
        return Ok(GraphqlField::new(
            "extension",
            compile_extension_args(stage, parsed.has_limit_stage.then_some(parsed.limit)),
            Vec::new(),
        ));
    }

    bail!("the GraphQL compiler could not determine a queryable leaf stage")
}

fn compile_project_stage_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: RegisteredStageKind<'_>,
) -> Result<GraphqlField> {
    match registered_stage {
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
        RegisteredStageKind::Extension(stage) => Ok(GraphqlField::new(
            "extension",
            compile_extension_args(stage, parsed.has_limit_stage.then_some(parsed.limit)),
            Vec::new(),
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

fn compile_artefacts_leaf(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> Result<GraphqlField> {
    if matches!(registered_stage, Some(RegisteredStageKind::Knowledge(_))) {
        bail!("knowledge() cannot be nested under artefacts() when compiling to GraphQL");
    }

    let mut node_selections =
        scalar_selections_for_leaf(SelectableLeaf::Artefact, &parsed.select_fields)?;

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
                scalar_selections_for_leaf(SelectableLeaf::Clone, &[])?,
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
            RegisteredStageKind::Extension(stage) => node_selections.push(
                GraphqlField::new(
                    "extension",
                    compile_extension_args(stage, parsed.has_limit_stage.then_some(parsed.limit)),
                    Vec::new(),
                )
                .into(),
            ),
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

fn wrap_in_scopes(parsed: &ParsedDevqlQuery, terminal_field: GraphqlField) -> GraphqlField {
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

    if let Some(project_path) = parsed.project_path.as_deref() {
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

fn compile_artefact_args(
    parsed: &ParsedDevqlQuery,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(filter) = compile_artefact_filter_input(parsed)? {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    Ok(args)
}

fn compile_checkpoint_args(parsed: &ParsedDevqlQuery) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(agent) = parsed.checkpoints.agent.as_deref() {
        args.push(GraphqlArgument::new("agent", quote_graphql_string(agent)));
    }
    if let Some(since) = parsed.checkpoints.since.as_deref() {
        args.push(GraphqlArgument::new(
            "since",
            compile_datetime_literal(since)?,
        ));
    }
    args.extend(first_arg(parsed.has_limit_stage.then_some(parsed.limit)));
    Ok(args)
}

fn compile_telemetry_args(parsed: &ParsedDevqlQuery) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(event_type) = parsed.telemetry.event_type.as_deref() {
        args.push(GraphqlArgument::new(
            "eventType",
            quote_graphql_string(event_type),
        ));
    }
    if let Some(agent) = parsed.telemetry.agent.as_deref() {
        args.push(GraphqlArgument::new("agent", quote_graphql_string(agent)));
    }
    if let Some(since) = parsed.telemetry.since.as_deref() {
        args.push(GraphqlArgument::new(
            "since",
            compile_datetime_literal(since)?,
        ));
    }
    args.extend(first_arg(parsed.has_limit_stage.then_some(parsed.limit)));
    Ok(args)
}

fn compile_deps_args(parsed: &ParsedDevqlQuery, first: Option<usize>) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(filter) = compile_deps_filter_input(parsed) {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    args
}

fn compile_clones_args(parsed: &ParsedDevqlQuery, first: Option<usize>) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(filter) = compile_clones_filter_input(parsed) {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    args
}

fn compile_knowledge_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(provider) = stage.args.get("provider") {
        args.push(GraphqlArgument::new("provider", enum_literal(provider)));
    }
    args.extend(first_arg(first));
    Ok(args)
}

fn compile_tests_args(
    parsed: &ParsedDevqlQuery,
    stage: &RegisteredStageCall,
    include_filter: bool,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if include_filter && let Some(filter) = compile_artefact_filter_input(parsed)? {
        args.push(GraphqlArgument::new("filter", filter));
    }
    if let Some(min_confidence) = stage.args.get("min_confidence") {
        args.push(GraphqlArgument::new(
            "minConfidence",
            min_confidence.clone(),
        ));
    }
    if let Some(linkage_source) = stage.args.get("linkage_source") {
        args.push(GraphqlArgument::new(
            "linkageSource",
            quote_graphql_string(linkage_source),
        ));
    }
    args.extend(first_arg(first));
    Ok(args)
}

fn compile_coverage_args(
    parsed: &ParsedDevqlQuery,
    include_filter: bool,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if include_filter && let Some(filter) = compile_artefact_filter_input(parsed)? {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    Ok(args)
}

fn compile_extension_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
) -> Vec<GraphqlArgument> {
    let mut args = vec![GraphqlArgument::new(
        "stage",
        quote_graphql_string(&stage.stage_name),
    )];
    if let Some(json_args) = compile_extension_stage_args(stage) {
        args.push(GraphqlArgument::new("args", json_args));
    }
    args.extend(first_arg(first));
    args
}

fn compile_artefact_filter_input(parsed: &ParsedDevqlQuery) -> Result<Option<String>> {
    let mut fields = Vec::new();
    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        fields.push(format!("kind: {}", enum_literal(kind)));
    }
    if let Some(symbol_fqn) = parsed.artefacts.symbol_fqn.as_deref() {
        fields.push(format!("symbolFqn: {}", quote_graphql_string(symbol_fqn)));
    }
    if let Some((start, end)) = parsed.artefacts.lines {
        fields.push(format!("lines: {{ start: {start}, end: {end} }}"));
    }
    if let Some(agent) = parsed.artefacts.agent.as_deref() {
        fields.push(format!("agent: {}", quote_graphql_string(agent)));
    }
    if let Some(since) = parsed.artefacts.since.as_deref() {
        fields.push(format!("since: {}", compile_datetime_literal(since)?));
    }

    Ok((!fields.is_empty()).then(|| format!("{{ {} }}", fields.join(", "))))
}

fn compile_deps_filter_input(parsed: &ParsedDevqlQuery) -> Option<String> {
    let mut fields = Vec::new();
    if let Some(kind) = parsed.deps.kind {
        fields.push(format!("kind: {}", enum_literal(kind.as_str())));
    }
    fields.push(format!(
        "direction: {}",
        enum_literal(parsed.deps.direction.as_str())
    ));
    if parsed.deps.include_unresolved {
        fields.push("includeUnresolved: true".to_string());
    }

    (!fields.is_empty()).then(|| format!("{{ {} }}", fields.join(", ")))
}

fn compile_clones_filter_input(parsed: &ParsedDevqlQuery) -> Option<String> {
    let mut fields = Vec::new();
    if let Some(relation_kind) = parsed.clones.relation_kind.as_deref() {
        fields.push(format!(
            "relationKind: {}",
            quote_graphql_string(relation_kind)
        ));
    }
    if let Some(min_score) = parsed.clones.min_score {
        fields.push(format!("minScore: {min_score}"));
    }

    (!fields.is_empty()).then(|| format!("{{ {} }}", fields.join(", ")))
}

fn compile_extension_stage_args(stage: &RegisteredStageCall) -> Option<String> {
    (!stage.args.is_empty()).then(|| {
        let pairs = stage
            .args
            .iter()
            .map(|(key, value)| format!("{key}: {}", compile_stage_json_literal(value)))
            .collect::<Vec<_>>();
        format!("{{ {} }}", pairs.join(", "))
    })
}

fn connection_field(
    name: &str,
    args: Vec<GraphqlArgument>,
    node_selection: Vec<GraphqlSelection>,
) -> GraphqlField {
    GraphqlField::new(
        name,
        args,
        vec![
            GraphqlField::new(
                "edges",
                Vec::new(),
                vec![GraphqlField::new("node", Vec::new(), node_selection).into()],
            )
            .into(),
        ],
    )
}

fn first_arg(first: Option<usize>) -> Vec<GraphqlArgument> {
    first
        .map(|value| vec![GraphqlArgument::new("first", value.to_string())])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_devql_pipeline_supports_project_stage_and_explicit_limit() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->project("packages/api")->artefacts()->limit(25)"#,
        )
        .expect("query parses");

        assert_eq!(parsed.project_path.as_deref(), Some("packages/api"));
        assert_eq!(parsed.limit, 25);
        assert!(parsed.has_limit_stage);
    }

    #[test]
    fn compile_project_asof_artefacts_pipeline() {
        let parsed = parse_devql_query(
            r#"repo("monorepo")->project("packages/api")->asOf(ref:"main")->artefacts(kind:"function")->limit(50)"#,
        )
        .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "monorepo") {
    project(path: "packages/api") {
      asOf(input: { ref: "main" }) {
        artefacts(filter: { kind: FUNCTION }, first: 50) {
          edges {
            node {
              id
              path
              symbolFqn
              canonicalKind
              languageKind
              startLine
              endLine
              language
            }
          }
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_file_artefacts_with_chat_history_enrichment() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->file("src/main.rs")->artefacts(lines:1..20,kind:"function")->chatHistory()->limit(5)"#,
        )
        .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "bitloops-cli") {
    file(path: "src/main.rs") {
      artefacts(filter: { kind: FUNCTION, lines: { start: 1, end: 20 } }) {
        edges {
          node {
            id
            path
            symbolFqn
            canonicalKind
            languageKind
            startLine
            endLine
            language
            chatHistory(first: 5) {
              edges {
                node {
                  sessionId
                  agent
                  timestamp
                  role
                  content
                }
              }
            }
          }
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_project_deps_pipeline() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->project("packages/api")->deps(kind:"imports",direction:"out")->limit(100)"#,
        )
        .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "bitloops-cli") {
    project(path: "packages/api") {
      deps(filter: { kind: IMPORTS, direction: OUT, includeUnresolved: true }, first: 100) {
        edges {
          node {
            id
            edgeKind
            fromArtefactId
            toArtefactId
            toSymbolRef
            startLine
            endLine
          }
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_repository_knowledge_pipeline() {
        let parsed = parse_devql_query(r#"repo("bitloops-cli")->knowledge()->limit(10)"#)
            .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "bitloops-cli") {
    knowledge(first: 10) {
      edges {
        node {
          id
          provider
          sourceKind
          canonicalExternalId
          externalUrl
          title
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_project_coverage_stage_with_filter() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->project("packages/api")->artefacts(kind:"function")->coverage()->limit(25)"#,
        )
        .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "bitloops-cli") {
    project(path: "packages/api") {
      coverage(filter: { kind: FUNCTION }, first: 25) {
        artefact {
          artefactId
          name
          kind
          filePath
          startLine
          endLine
        }
        coverage {
          coverageSource
          lineCoveragePct
          branchCoveragePct
          lineDataAvailable
          branchDataAvailable
          uncoveredLines
          branches {
            line
            block
            branch
            covered
            hitCount
          }
        }
        summary {
          uncoveredLineCount
          uncoveredBranchCount
          diagnosticCount
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_select_fields_to_graphql_names() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->artefacts(agent:"claude-code")->select(path,canonical_kind,symbol_fqn,start_line,end_line)->limit(50)"#,
        )
        .expect("query parses");

        let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

        assert_eq!(
            graphql,
            r#"query {
  repo(name: "bitloops-cli") {
    artefacts(filter: { agent: "claude-code" }, first: 50) {
      edges {
        node {
          path
          canonicalKind
          symbolFqn
          startLine
          endLine
        }
      }
    }
  }
}"#
        );
    }

    #[test]
    fn compile_rejects_unknown_select_field() {
        let parsed =
            parse_devql_query(r#"repo("bitloops-cli")->artefacts()->select(unknown_field)"#)
                .expect("query parses");

        let err = compile_devql_to_graphql(&parsed).expect_err("unknown field must fail");

        assert!(
            err.to_string()
                .contains("unsupported select() field `unknown_field`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn compile_rejects_multiple_registered_stages() {
        let parsed = parse_devql_query(
            r#"repo("bitloops-cli")->artefacts(kind:"function")->tests()->coverage()->limit(5)"#,
        )
        .expect("query parses");

        let err = compile_devql_to_graphql(&parsed).expect_err("multiple stages must fail");

        assert!(
            err.to_string()
                .contains("does not yet support multiple registered capability-pack stages"),
            "unexpected error: {err}"
        );
    }
}
