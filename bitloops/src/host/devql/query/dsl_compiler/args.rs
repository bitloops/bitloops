use anyhow::Result;

use super::document_builder::{GraphqlArgument, GraphqlField, GraphqlSelection};
use super::field_mapping::{compile_datetime_literal, enum_literal, quote_graphql_string};
use super::types::DepsSummaryStageSpec;
use super::{ParsedDevqlQuery, RegisteredStageCall};

pub(super) fn compile_artefact_args(
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

pub(super) fn compile_checkpoint_args(parsed: &ParsedDevqlQuery) -> Result<Vec<GraphqlArgument>> {
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

pub(super) fn compile_telemetry_args(parsed: &ParsedDevqlQuery) -> Result<Vec<GraphqlArgument>> {
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

pub(super) fn compile_deps_args(
    parsed: &ParsedDevqlQuery,
    first: Option<usize>,
) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(filter) = compile_deps_filter_input(parsed) {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    args
}

pub(super) fn compile_clones_args(
    parsed: &ParsedDevqlQuery,
    first: Option<usize>,
) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(filter) = compile_clones_filter_input(parsed) {
        args.push(GraphqlArgument::new("filter", filter));
    }
    args.extend(first_arg(first));
    args
}

pub(super) fn compile_clone_summary_args(
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(filter) = compile_artefact_filter_input(parsed)? {
        args.push(GraphqlArgument::new("filter", filter));
    }
    if let Some(clone_filter) = compile_clones_filter_input(parsed) {
        args.push(GraphqlArgument::new("cloneFilter", clone_filter));
    }
    Ok(args)
}

pub(super) fn compile_deps_summary_args(spec: DepsSummaryStageSpec) -> Vec<GraphqlArgument> {
    let mut fields = Vec::new();
    if let Some(kind) = spec.kind {
        let kind = match kind {
            super::super::DepsKind::Imports => "imports",
            super::super::DepsKind::Calls => "calls",
            super::super::DepsKind::References => "references",
            super::super::DepsKind::Extends => "extends",
            super::super::DepsKind::Implements => "implements",
            super::super::DepsKind::Exports => "exports",
        };
        fields.push(format!("kind: {}", enum_literal(kind)));
    }
    if let Some(direction) = spec.direction {
        let direction = match direction {
            super::super::DepsDirection::Out => "out",
            super::super::DepsDirection::In => "in",
            super::super::DepsDirection::Both => "both",
        };
        fields.push(format!("direction: {}", enum_literal(direction)));
    }
    fields.push(format!(
        "unresolved: {}",
        if spec.unresolved.unwrap_or(false) {
            "true"
        } else {
            "false"
        }
    ));

    vec![GraphqlArgument::new(
        "filter",
        format!("{{ {} }}", fields.join(", ")),
    )]
}

pub(super) fn compile_knowledge_args(
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

pub(super) fn compile_tests_args(
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

pub(super) fn compile_coverage_args(
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

pub(super) fn compile_artefact_filter_input(parsed: &ParsedDevqlQuery) -> Result<Option<String>> {
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

pub(super) fn compile_deps_filter_input(parsed: &ParsedDevqlQuery) -> Option<String> {
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

pub(super) fn compile_clones_filter_input(parsed: &ParsedDevqlQuery) -> Option<String> {
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
    if let Some(neighbors) = parsed.clones.neighbors {
        let clamped = neighbors.clamp(
            crate::capability_packs::semantic_clones::scoring::MIN_ANN_NEIGHBORS as i64,
            crate::capability_packs::semantic_clones::scoring::MAX_ANN_NEIGHBORS as i64,
        );
        fields.push(format!("neighbors: {clamped}"));
    }

    (!fields.is_empty()).then(|| format!("{{ {} }}", fields.join(", ")))
}

pub(super) fn connection_field(
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

pub(super) fn first_arg(first: Option<usize>) -> Vec<GraphqlArgument> {
    first
        .map(|value| vec![GraphqlArgument::new("first", value.to_string())])
        .unwrap_or_default()
}
