use anyhow::{Result, bail};

use super::field_mapping::{
    KNOWLEDGE_STAGE_NAME, TESTS_SUMMARY_STAGE_NAME, is_coverage_stage_name, is_tests_stage_name,
};
use super::{GraphqlCompileMode, ParsedDevqlQuery, RegisteredStageKind};

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

pub(super) fn validate_graphql_compiler_support(
    parsed: &ParsedDevqlQuery,
    registered_stage: Option<RegisteredStageKind<'_>>,
    mode: GraphqlCompileMode,
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
    let has_tests_summary_stage =
        matches!(registered_stage, Some(RegisteredStageKind::TestsSummary));

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
            Some(RegisteredStageKind::Knowledge(_)) | Some(RegisteredStageKind::TestsSummary)
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
        | RegisteredStageKind::TestsSummary => parsed.project_path.is_some(),
        RegisteredStageKind::Knowledge(_) => false,
    }
}
