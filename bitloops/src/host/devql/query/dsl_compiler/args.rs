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

pub(super) fn compile_select_artefacts_args(
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<GraphqlArgument>> {
    let Some(selector) = parsed.select_artefacts.as_ref() else {
        return Ok(Vec::new());
    };

    let mut fields = Vec::new();
    if let Some(symbol_fqn) = selector.symbol_fqn.as_deref() {
        fields.push(format!("symbolFqn: {}", quote_graphql_string(symbol_fqn)));
    }
    if let Some(search) = selector.search.as_deref() {
        fields.push(format!("search: {}", quote_graphql_string(search)));
    }
    if let Some(search_mode) = selector.search_mode.as_deref() {
        fields.push(format!("searchMode: {}", enum_literal(search_mode)));
    }
    if let Some(path) = selector.path.as_deref() {
        fields.push(format!("path: {}", quote_graphql_string(path)));
    }
    if let Some((start, end)) = selector.lines {
        fields.push(format!("lines: {{ start: {start}, end: {end} }}"));
    }

    Ok(vec![GraphqlArgument::new(
        "by",
        format!("{{ {} }}", fields.join(", ")),
    )])
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

pub(super) fn compile_selection_checkpoint_args(
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<GraphqlArgument>> {
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
    Ok(args)
}

pub(super) fn compile_selection_historical_context_args(
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(agent) = parsed.historical_context.agent.as_deref() {
        args.push(GraphqlArgument::new("agent", quote_graphql_string(agent)));
    }
    if let Some(since) = parsed.historical_context.since.as_deref() {
        args.push(GraphqlArgument::new(
            "since",
            compile_datetime_literal(since)?,
        ));
    }
    if let Some(evidence_kind) = parsed.historical_context.evidence_kind.as_deref() {
        args.push(GraphqlArgument::new(
            "evidenceKind",
            enum_literal(evidence_kind),
        ));
    }
    Ok(args)
}

pub(super) fn compile_selection_context_guidance_args(
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = Vec::new();
    if let Some(agent) = parsed.context_guidance.agent.as_deref() {
        args.push(GraphqlArgument::new("agent", quote_graphql_string(agent)));
    }
    if let Some(since) = parsed.context_guidance.since.as_deref() {
        args.push(GraphqlArgument::new(
            "since",
            compile_datetime_literal(since)?,
        ));
    }
    if let Some(evidence_kind) = parsed.context_guidance.evidence_kind.as_deref() {
        args.push(GraphqlArgument::new(
            "evidenceKind",
            enum_literal(evidence_kind),
        ));
    }
    if let Some(category) = parsed.context_guidance.category.as_deref() {
        args.push(GraphqlArgument::new("category", enum_literal(category)));
    }
    if let Some(kind) = parsed.context_guidance.kind.as_deref() {
        args.push(GraphqlArgument::new("kind", quote_graphql_string(kind)));
    }
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

pub(super) fn compile_selection_deps_args(parsed: &ParsedDevqlQuery) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(kind) = parsed.deps.kind {
        args.push(GraphqlArgument::new("kind", enum_literal(kind.as_str())));
    }
    if parsed.deps.direction != super::super::DepsDirection::Both {
        args.push(GraphqlArgument::new(
            "direction",
            enum_literal(parsed.deps.direction.as_str()),
        ));
    }
    args.push(GraphqlArgument::new(
        "includeUnresolved",
        if parsed.deps.include_unresolved {
            "true"
        } else {
            "false"
        },
    ));
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

pub(super) fn compile_selection_clones_args(parsed: &ParsedDevqlQuery) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(relation_kind) = parsed.clones.relation_kind.as_deref() {
        args.push(GraphqlArgument::new(
            "relationKind",
            quote_graphql_string(relation_kind),
        ));
    }
    if let Some(min_score) = parsed.clones.min_score {
        args.push(GraphqlArgument::new("minScore", min_score.to_string()));
    }
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

pub(super) fn compile_selection_tests_args(stage: &RegisteredStageCall) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
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
    args
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

pub(super) fn compile_http_search_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let mut args = vec![GraphqlArgument::new(
        "terms",
        compile_terms_list(required_stage_arg(stage, "terms")?),
    )];
    args.extend(first_arg(first));
    Ok(args)
}

pub(super) fn compile_http_header_producers_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
) -> Result<Vec<GraphqlArgument>> {
    let header = stage
        .args
        .get("header")
        .or_else(|| stage.args.get("header_name"))
        .or_else(|| stage.args.get("headerName"))
        .ok_or_else(|| anyhow::anyhow!("httpHeaderProducers() requires header:"))?;
    let mut args = vec![GraphqlArgument::new(
        "headerName",
        quote_graphql_string(header),
    )];
    args.extend(first_arg(first));
    Ok(args)
}

pub(super) fn compile_http_lifecycle_boundaries_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if let Some(terms) = stage.args.get("terms") {
        args.push(GraphqlArgument::new("terms", compile_terms_list(terms)));
    }
    args.extend(first_arg(first));
    args
}

pub(super) fn compile_http_lossy_transforms_args(
    stage: &RegisteredStageCall,
    first: Option<usize>,
    include_around: bool,
) -> Vec<GraphqlArgument> {
    let mut args = Vec::new();
    if include_around {
        let mut around_fields = Vec::new();
        if let Some(symbol_fqn) = stage
            .args
            .get("symbol_fqn")
            .or_else(|| stage.args.get("symbolFqn"))
        {
            around_fields.push(format!("symbolFqn: {}", quote_graphql_string(symbol_fqn)));
        }
        if let Some(symbol_id) = stage
            .args
            .get("symbol_id")
            .or_else(|| stage.args.get("symbolId"))
        {
            around_fields.push(format!("symbolId: {}", quote_graphql_string(symbol_id)));
        }
        if let Some(artefact_id) = stage
            .args
            .get("artefact_id")
            .or_else(|| stage.args.get("artefactId"))
        {
            around_fields.push(format!("artefactId: {}", quote_graphql_string(artefact_id)));
        }
        if let Some(path) = stage.args.get("path") {
            around_fields.push(format!("path: {}", quote_graphql_string(path)));
        }
        if !around_fields.is_empty() {
            args.push(GraphqlArgument::new(
                "around",
                format!("{{ {} }}", around_fields.join(", ")),
            ));
        }
    }
    args.extend(first_arg(first));
    args
}

pub(super) fn compile_http_patch_impact_args(
    stage: &RegisteredStageCall,
) -> Result<Vec<GraphqlArgument>> {
    let fingerprint = stage
        .args
        .get("patch_fingerprint")
        .or_else(|| stage.args.get("patchFingerprint"))
        .ok_or_else(|| anyhow::anyhow!("httpPatchImpact() requires patch_fingerprint:"))?;
    Ok(vec![GraphqlArgument::new(
        "input",
        format!(
            "{{ patchFingerprint: {} }}",
            quote_graphql_string(fingerprint)
        ),
    )])
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
    fields.push(format!(
        "includeUnresolved: {}",
        if parsed.deps.include_unresolved {
            "true"
        } else {
            "false"
        }
    ));

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

fn required_stage_arg<'a>(stage: &'a RegisteredStageCall, name: &str) -> Result<&'a str> {
    stage
        .args
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("{}() requires {name}:", stage.stage_name))
}

fn compile_terms_list(raw: &str) -> String {
    let terms = raw
        .split(',')
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(quote_graphql_string)
        .collect::<Vec<_>>();
    format!("[{}]", terms.join(", "))
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
