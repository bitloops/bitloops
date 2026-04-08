use anyhow::{Result, anyhow, bail};

use super::field_mapping::{
    CLONE_SUMMARY_STAGE_NAME, KNOWLEDGE_STAGE_NAME, TESTS_SUMMARY_STAGE_NAME,
    is_coverage_stage_name, is_tests_stage_name,
};
use super::types::DepsSummaryStageSpec;
use super::{
    GraphqlCompileMode, ParsedDevqlQuery, RegisteredStageCall, RegisteredStageKind,
    SelectArtefactsFilter,
};

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

    bail!(
        "the GraphQL compiler does not support capability-pack stage `{}`; register an explicit typed GraphQL/DSL contribution",
        stage.stage_name
    )
}

fn resolve_summary_stage_kind(stage: &RegisteredStageCall) -> Result<RegisteredStageKind<'_>> {
    let Some(deps_arg) = stage.args.get("deps") else {
        if !stage.args.is_empty() {
            bail!(
                "summary() for clones does not accept arguments; use summary(deps:true, ...) for dependency summary"
            );
        }
        return Ok(RegisteredStageKind::CloneSummary);
    };

    let deps_enabled = super::super::parse_bool_literal("summary deps", deps_arg)?;
    if !deps_enabled {
        bail!("summary(deps:...) requires deps:true");
    }

    for key in stage.args.keys() {
        match key.as_str() {
            "deps" | "kind" | "direction" | "unresolved" => {}
            _ => {
                bail!(
                    "summary(deps:true, ...) received unsupported argument `{key}`; allowed args: deps, kind, direction, unresolved"
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

        if parsed.has_limit_stage {
            bail!("selectArtefacts(...) does not support limit(); use the stage defaults in v1");
        }

        let terminal_stage_count = usize::from(parsed.has_checkpoints_stage)
            + usize::from(parsed.has_clones_stage)
            + usize::from(parsed.has_deps_stage)
            + usize::from(matches!(
                registered_stage,
                Some(RegisteredStageKind::Tests(_))
            ));
        if terminal_stage_count == 0 {
            bail!("selectArtefacts(...) requires checkpoints(), clones(), deps(), or tests()");
        }
        if terminal_stage_count > 1 {
            bail!("selectArtefacts(...) supports exactly one terminal stage in v1");
        }

        if matches!(
            registered_stage,
            Some(
                RegisteredStageKind::Coverage
                    | RegisteredStageKind::CloneSummary
                    | RegisteredStageKind::DepsSummary(_)
                    | RegisteredStageKind::TestsSummary
                    | RegisteredStageKind::Knowledge(_)
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
    let has_clone_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::CloneSummary));
    let has_deps_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::DepsSummary(_)));
    let has_tests_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::TestsSummary));

    if has_clone_summary_stage && !parsed.has_clones_stage {
        bail!("summary() requires a clones() stage in the query");
    }

    if has_clone_summary_stage && !parsed.select_fields.is_empty() {
        bail!("summary() does not support select() in the GraphQL compiler yet");
    }

    if has_deps_summary_stage && !parsed.has_artefacts_stage {
        bail!("summary(deps:true, ...) requires an artefacts() stage in the query");
    }

    if has_deps_summary_stage && parsed.has_deps_stage {
        bail!("summary(deps:true, ...) cannot be combined with deps() stage");
    }

    if has_deps_summary_stage && parsed.has_clones_stage {
        bail!("summary(deps:true, ...) cannot be combined with clones() stage");
    }

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

    if has_tests_summary_stage && parsed.has_artefacts_stage {
        bail!("test_harness_tests_summary() does not support artefacts() traversal");
    }

    if has_tests_summary_stage && parsed.has_deps_stage {
        bail!("test_harness_tests_summary() cannot be combined with deps() stage");
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

    if parsed.has_deps_stage && parsed.has_artefacts_stage {
        bail!("deps() after artefacts() is not yet supported by the GraphQL compiler");
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

    if parsed.has_artefacts_stage
        || parsed.has_checkpoints_stage
        || parsed.has_telemetry_stage
        || parsed.has_deps_stage
        || matches!(
            registered_stage,
            Some(RegisteredStageKind::Knowledge(_))
                | Some(RegisteredStageKind::TestsSummary)
                | Some(RegisteredStageKind::DepsSummary(_))
        )
    {
        return Ok(());
    }

    bail!("the GraphQL compiler could not determine a queryable leaf stage")
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
    }
}

fn validate_select_artefacts_selector(selector: &SelectArtefactsFilter) -> Result<()> {
    let symbol_fqn = selector
        .symbol_fqn
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let path = selector
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (symbol_fqn, path, selector.lines) {
        (Some(_), None, None) => Ok(()),
        (None, Some(_), None | Some(_)) => Ok(()),
        (None, None, Some(_)) => {
            bail!("selectArtefacts(...) requires path: when lines: is provided")
        }
        (Some(_), Some(_), _) | (Some(_), None, Some(_)) => {
            bail!("selectArtefacts(...) allows either symbol_fqn: or path:/lines:, not both")
        }
        (None, None, None) => bail!("selectArtefacts(...) requires symbol_fqn: or path:"),
    }
}
