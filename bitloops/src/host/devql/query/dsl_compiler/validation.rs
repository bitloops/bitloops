use anyhow::{Result, anyhow, bail};

use super::field_mapping::{
    CLONE_SUMMARY_STAGE_NAME, KNOWLEDGE_STAGE_NAME, TESTS_SUMMARY_STAGE_NAME,
    is_coverage_stage_name, is_tests_stage_name,
};
use super::types::DepsSummaryStageSpec;
use super::{
    ContextGuidanceFilter, GraphqlCompileMode, HistoricalContextFilter, ParsedDevqlQuery,
    RegisteredStageCall, RegisteredStageKind, SelectArtefactsFilter,
};

const CONTEXT_GUIDANCE_CATEGORIES: &[&str] = &[
    "decision",
    "constraint",
    "pattern",
    "risk",
    "verification",
    "context",
];
const CONTEXT_GUIDANCE_EVIDENCE_KINDS: &[&str] =
    &["symbol_provenance", "file_relation", "line_overlap"];

pub(super) fn resolve_registered_stage(
    parsed: &ParsedDevqlQuery,
) -> Result<Option<RegisteredStageKind<'_>>> {
    if parsed.registered_stages.len() > 1 {
        bail!(
            "the GraphQL compiler does not yet support multiple registered capability-pack stages in one query"
        );
    }

    let Some(stage) = parsed.registered_stages.first() else {
        return Ok(None);
    };

    if stage.stage_name == CLONE_SUMMARY_STAGE_NAME {
        return Ok(Some(resolve_summary_stage_kind(stage)?));
    }
    if is_tests_stage_name(&stage.stage_name) {
        return Ok(Some(RegisteredStageKind::Tests(stage)));
    }
    if is_coverage_stage_name(&stage.stage_name) {
        return Ok(Some(RegisteredStageKind::Coverage));
    }
    if stage.stage_name == TESTS_SUMMARY_STAGE_NAME {
        return Ok(Some(RegisteredStageKind::TestsSummary));
    }
    if stage.stage_name == KNOWLEDGE_STAGE_NAME {
        return Ok(Some(RegisteredStageKind::Knowledge(stage)));
    }
    match stage.stage_name.as_str() {
        "overview" => return Ok(Some(RegisteredStageKind::SelectionOverview)),
        "httpSearch" | "http_search" => return Ok(Some(RegisteredStageKind::HttpSearch(stage))),
        "httpContext" | "http_context" => {
            return Ok(Some(RegisteredStageKind::HttpContext(stage)));
        }
        "httpHeaderProducers" | "http_header_producers" => {
            return Ok(Some(RegisteredStageKind::HttpHeaderProducers(stage)));
        }
        "httpLifecycleBoundaries" | "http_lifecycle_boundaries" => {
            return Ok(Some(RegisteredStageKind::HttpLifecycleBoundaries(stage)));
        }
        "httpLossyTransforms" | "http_lossy_transforms" => {
            return Ok(Some(RegisteredStageKind::HttpLossyTransforms(stage)));
        }
        "httpPatchImpact" | "http_patch_impact" => {
            return Ok(Some(RegisteredStageKind::HttpPatchImpact(stage)));
        }
        _ => {}
    }

    bail!(
        "the GraphQL compiler does not support capability-pack stage `{}`; register an explicit typed GraphQL/DSL contribution",
        stage.stage_name
    )
}

fn resolve_summary_stage_kind(stage: &RegisteredStageCall) -> Result<RegisteredStageKind<'_>> {
    let Some(deps_arg) = stage.args.get("dependencies") else {
        if !stage.args.is_empty() {
            bail!(
                "summary() for clones does not accept arguments; use summary(dependencies:true, ...) for dependency summary"
            );
        }
        return Ok(RegisteredStageKind::CloneSummary);
    };

    let deps_enabled = super::super::parse_bool_literal("summary dependencies", deps_arg)?;
    if !deps_enabled {
        bail!("summary(dependencies:...) requires dependencies:true");
    }

    for key in stage.args.keys() {
        match key.as_str() {
            "dependencies" | "kind" | "direction" | "unresolved" => {}
            _ => {
                bail!(
                    "summary(dependencies:true, ...) received unsupported argument `{key}`; allowed args: dependencies, kind, direction, unresolved"
                );
            }
        }
    }

    let kind = if let Some(kind) = stage.args.get("kind") {
        Some(super::super::DepsKind::from_str(kind).ok_or_else(|| {
            anyhow::anyhow!(
                "summary(kind:...) must be one of: {}",
                super::super::DepsKind::all_names().join(", ")
            )
        })?)
    } else {
        None
    };

    let direction = if let Some(direction) = stage.args.get("direction") {
        Some(
            super::super::DepsDirection::from_str(direction).ok_or_else(|| {
                anyhow::anyhow!(
                    "summary(direction:...) must be one of: {}",
                    super::super::DepsDirection::all_names().join(", ")
                )
            })?,
        )
    } else {
        None
    };

    let unresolved = if let Some(unresolved) = stage.args.get("unresolved") {
        Some(parse_summary_unresolved_flag(unresolved)?)
    } else {
        None
    };

    Ok(RegisteredStageKind::DepsSummary(DepsSummaryStageSpec {
        kind,
        direction,
        unresolved,
    }))
}

fn parse_summary_unresolved_flag(value: &str) -> Result<bool> {
    super::super::parse_bool_literal("summary unresolved", value)
        .map_err(|_| anyhow!("summary(unresolved:...) must be boolean true/false"))
}

pub(super) fn validate_graphql_compiler_support(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
    mode: GraphqlCompileMode,
) -> Result<()> {
    if let Some(selector) = parsed.select_artefacts.as_ref() {
        validate_select_artefacts_selector(selector)?;

        if mode != GraphqlCompileMode::Slim {
            bail!(
                "selectArtefacts(...) is repo-scoped slim only; compile it against the slim DevQL endpoint"
            );
        }

        if parsed.file.is_some() || parsed.files_path.is_some() {
            bail!("selectArtefacts(...) cannot be combined with file() or files()");
        }

        if parsed.as_of.is_some() {
            bail!("selectArtefacts(...) does not support asOf(...) in v1");
        }

        if parsed.has_artefacts_stage || parsed.has_chat_history_stage || parsed.has_telemetry_stage
        {
            bail!(
                "selectArtefacts(...) cannot be combined with artefacts(), chatHistory(), or telemetry()"
            );
        }

        if parsed.has_limit_stage
            && !matches!(registered_stage, Some(RegisteredStageKind::HttpContext(_)))
        {
            bail!(
                "selectArtefacts(...) only supports limit() with httpContext() in the GraphQL compiler"
            );
        }

        let terminal_stage_count = usize::from(parsed.has_checkpoints_stage)
            + usize::from(parsed.has_historical_context_stage)
            + usize::from(parsed.has_context_guidance_stage)
            + usize::from(parsed.has_clones_stage)
            + usize::from(parsed.has_deps_stage)
            + usize::from(matches!(
                registered_stage,
                Some(
                    RegisteredStageKind::Tests(_)
                        | RegisteredStageKind::SelectionOverview
                        | RegisteredStageKind::HttpContext(_)
                )
            ));
        if terminal_stage_count == 0 {
            bail!(
                "selectArtefacts(...) requires overview(), checkpoints(), historicalContext(), contextGuidance(), clones(), dependencies(), tests(), or httpContext()"
            );
        }
        if terminal_stage_count > 1 {
            bail!("selectArtefacts(...) supports exactly one terminal stage in v1");
        }

        validate_historical_context_filter(&parsed.historical_context)?;
        validate_context_guidance_filter(&parsed.context_guidance)?;

        if matches!(
            registered_stage,
            Some(
                RegisteredStageKind::Coverage
                    | RegisteredStageKind::CloneSummary
                    | RegisteredStageKind::DepsSummary(_)
                    | RegisteredStageKind::TestsSummary
                    | RegisteredStageKind::Knowledge(_)
                    | RegisteredStageKind::HttpSearch(_)
                    | RegisteredStageKind::HttpHeaderProducers(_)
                    | RegisteredStageKind::HttpLifecycleBoundaries(_)
                    | RegisteredStageKind::HttpLossyTransforms(_)
                    | RegisteredStageKind::HttpPatchImpact(_)
            )
        ) {
            bail!("selectArtefacts(...) does not support that registered stage in v1");
        }

        return Ok(());
    }

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

    if parsed.has_historical_context_stage {
        bail!("historicalContext() is only supported after selectArtefacts(...) in v1");
    }

    if parsed.has_context_guidance_stage {
        bail!("contextGuidance() is only supported after selectArtefacts(...)");
    }

    if parsed.has_clones_stage && !parsed.has_artefacts_stage {
        bail!("clones() requires an artefacts() stage in the query");
    }

    if parsed.has_deps_stage && parsed.has_chat_history_stage {
        bail!("dependencies() cannot be combined with chatHistory() stage");
    }

    if parsed.has_clones_stage && parsed.has_deps_stage {
        bail!("clones() cannot be combined with dependencies() stage");
    }

    if parsed.has_chat_history_stage && (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
    {
        bail!("chatHistory() cannot be combined with checkpoints()/telemetry() stages");
    }

    if parsed.has_clones_stage && parsed.has_chat_history_stage {
        bail!("clones() cannot be combined with chatHistory() stage");
    }

    let has_tests_stage = matches!(registered_stage, Some(RegisteredStageKind::Tests(_)));
    let has_coverage_stage = matches!(registered_stage, Some(RegisteredStageKind::Coverage));
    let has_clone_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::CloneSummary));
    let has_deps_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::DepsSummary(_)));
    let has_tests_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::TestsSummary));
    let has_direct_http_stage = matches!(
        registered_stage,
        Some(
            RegisteredStageKind::HttpSearch(_)
                | RegisteredStageKind::HttpHeaderProducers(_)
                | RegisteredStageKind::HttpLifecycleBoundaries(_)
                | RegisteredStageKind::HttpLossyTransforms(_)
                | RegisteredStageKind::HttpPatchImpact(_)
        )
    );

    if has_clone_summary_stage && !parsed.has_clones_stage {
        bail!("summary() requires a clones() stage in the query");
    }

    if has_clone_summary_stage && !parsed.select_fields.is_empty() {
        bail!("summary() does not support select() in the GraphQL compiler yet");
    }

    if has_deps_summary_stage && !parsed.has_artefacts_stage {
        bail!("summary(dependencies:true, ...) requires an artefacts() stage in the query");
    }

    if has_deps_summary_stage && parsed.has_deps_stage {
        bail!("summary(dependencies:true, ...) cannot be combined with dependencies() stage");
    }

    if has_deps_summary_stage && parsed.has_clones_stage {
        bail!("summary(dependencies:true, ...) cannot be combined with clones() stage");
    }

    if has_tests_stage && !parsed.has_artefacts_stage {
        bail!("tests() requires an artefacts() stage in the query");
    }

    if has_tests_stage && parsed.has_deps_stage {
        bail!("tests() cannot be combined with dependencies() stage");
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
        bail!("coverage() cannot be combined with dependencies() stage");
    }

    if has_coverage_stage && parsed.has_clones_stage {
        bail!("coverage() cannot be combined with clones() stage");
    }

    if has_coverage_stage && parsed.has_chat_history_stage {
        bail!("coverage() cannot be combined with chatHistory() stage");
    }

    if has_tests_summary_stage && parsed.has_artefacts_stage {
        bail!("test_harness_tests_summary() does not support artefacts() traversal");
    }

    if has_tests_summary_stage && parsed.has_deps_stage {
        bail!("test_harness_tests_summary() cannot be combined with dependencies() stage");
    }

    if has_tests_summary_stage && parsed.has_clones_stage {
        bail!("test_harness_tests_summary() cannot be combined with clones() stage");
    }

    if has_tests_summary_stage && parsed.has_chat_history_stage {
        bail!("test_harness_tests_summary() cannot be combined with chatHistory() stage");
    }

    if has_tests_summary_stage && (parsed.file.is_some() || parsed.files_path.is_some()) {
        bail!("test_harness_tests_summary() does not support file() or files() scopes");
    }

    if matches!(
        registered_stage,
        Some(RegisteredStageKind::SelectionOverview)
    ) {
        bail!("overview() is only supported after selectArtefacts(...) in the GraphQL compiler");
    }

    if matches!(registered_stage, Some(RegisteredStageKind::HttpContext(_))) {
        bail!("httpContext() is only supported after selectArtefacts(...) in the DSL compiler");
    }

    if parsed.has_deps_stage && parsed.has_artefacts_stage {
        bail!("dependencies() after artefacts() is not yet supported by the GraphQL compiler");
    }

    if has_direct_http_stage
        && (parsed.has_checkpoints_stage
            || parsed.has_telemetry_stage
            || parsed.has_deps_stage
            || parsed.has_clones_stage
            || parsed.has_chat_history_stage)
    {
        bail!(
            "HTTP direct lookup stages cannot be combined with checkpoints(), telemetry(), dependencies(), clones(), or chatHistory()"
        );
    }

    if parsed.has_artefacts_stage
        && has_direct_http_stage
        && !matches!(
            registered_stage,
            Some(RegisteredStageKind::HttpLossyTransforms(_))
        )
    {
        bail!("only httpLossyTransforms() can be nested under artefacts() in the DSL compiler");
    }

    if mode == GraphqlCompileMode::Global
        && parsed.has_telemetry_stage
        && parsed.project_path.is_some()
    {
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
        bail!("dependencies() requires project(), file(), or files() when compiling to GraphQL");
    }

    if parsed.has_deps_stage
        && parsed.as_of.is_some()
        && parsed.file.is_none()
        && parsed.files_path.is_none()
    {
        bail!(
            "dependencies() with asOf(...) is only supported when further scoped through file() or files()"
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

    if parsed.has_artefacts_stage
        || parsed.has_checkpoints_stage
        || parsed.has_telemetry_stage
        || parsed.has_deps_stage
        || matches!(
            registered_stage,
            Some(RegisteredStageKind::Knowledge(_))
                | Some(RegisteredStageKind::TestsSummary)
                | Some(RegisteredStageKind::DepsSummary(_))
                | Some(RegisteredStageKind::HttpSearch(_))
                | Some(RegisteredStageKind::HttpHeaderProducers(_))
                | Some(RegisteredStageKind::HttpLifecycleBoundaries(_))
                | Some(RegisteredStageKind::HttpLossyTransforms(_))
                | Some(RegisteredStageKind::HttpPatchImpact(_))
        )
    {
        return Ok(());
    }

    bail!("the GraphQL compiler could not determine a queryable leaf stage")
}

fn validate_context_guidance_filter(filter: &ContextGuidanceFilter) -> Result<()> {
    if let Some(evidence_kind) = filter.evidence_kind.as_deref() {
        validate_choice(
            "contextGuidance(evidenceKind:...)",
            evidence_kind,
            CONTEXT_GUIDANCE_EVIDENCE_KINDS,
        )?;
    }

    if let Some(category) = filter.category.as_deref() {
        validate_choice(
            "contextGuidance(category:...)",
            category,
            CONTEXT_GUIDANCE_CATEGORIES,
        )?;
    }

    if filter
        .kind
        .as_deref()
        .is_some_and(|kind| kind.trim().is_empty())
    {
        bail!("contextGuidance(kind:...) must be non-empty");
    }

    Ok(())
}

fn validate_historical_context_filter(filter: &HistoricalContextFilter) -> Result<()> {
    if let Some(evidence_kind) = filter.evidence_kind.as_deref() {
        validate_choice(
            "historicalContext(evidenceKind:...)",
            evidence_kind,
            CONTEXT_GUIDANCE_EVIDENCE_KINDS,
        )?;
    }

    Ok(())
}

fn validate_choice(label: &str, value: &str, supported: &[&str]) -> Result<()> {
    let normalized = value.trim().to_ascii_lowercase();
    if supported.contains(&normalized.as_str()) {
        return Ok(());
    }

    bail!("{label} must be one of: {}", supported.join(", "))
}

pub(super) fn should_compile_project_stage(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
) -> bool {
    let Some(registered_stage) = registered_stage else {
        return false;
    };

    if parsed.as_of.is_some()
        || parsed.select_artefacts.is_some()
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
        RegisteredStageKind::CloneSummary => false,
        RegisteredStageKind::DepsSummary(_) => false,
        RegisteredStageKind::Tests(_)
        | RegisteredStageKind::Coverage
        | RegisteredStageKind::TestsSummary => parsed.project_path.is_some(),
        RegisteredStageKind::Knowledge(_) => false,
        RegisteredStageKind::SelectionOverview | RegisteredStageKind::HttpContext(_) => false,
        RegisteredStageKind::HttpSearch(_)
        | RegisteredStageKind::HttpHeaderProducers(_)
        | RegisteredStageKind::HttpLifecycleBoundaries(_)
        | RegisteredStageKind::HttpLossyTransforms(_)
        | RegisteredStageKind::HttpPatchImpact(_) => parsed.project_path.is_some(),
    }
}

fn validate_select_artefacts_selector(selector: &SelectArtefactsFilter) -> Result<()> {
    let symbol_fqn = selector
        .symbol_fqn
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let search = selector
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if selector
        .search
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("selectArtefacts(...) requires search: to be non-empty");
    }
    let path = selector
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let search_mode = selector
        .search_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let path_selector_requested = path.is_some() || selector.lines.is_some();
    let selector_count = usize::from(symbol_fqn.is_some())
        + usize::from(search.is_some())
        + usize::from(path_selector_requested);
    if selector_count == 0 {
        bail!("selectArtefacts(...) requires symbol_fqn:, search:, or path:");
    }
    if selector_count > 1 {
        bail!("selectArtefacts(...) allows exactly one of symbol_fqn:, search:, or path:/lines:");
    }
    if path_selector_requested && path.is_none() {
        bail!("selectArtefacts(...) requires path: when lines: is provided");
    }
    if selector
        .search_mode
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("selectArtefacts(...) requires search_mode: to be non-empty");
    }
    if search_mode.is_some() && search.is_none() {
        bail!("selectArtefacts(...) only allows search_mode: when search: is provided");
    }
    if let Some(search_mode) = search_mode {
        match search_mode.to_ascii_lowercase().as_str() {
            "auto" | "identity" | "code" | "summary" | "lexical" => {}
            _ => bail!(
                "selectArtefacts(search_mode:...) must be one of: auto, identity, code, summary, lexical"
            ),
        }
    }

    Ok(())
}
