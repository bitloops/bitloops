use super::*;
use anyhow::{anyhow, bail};

pub(crate) async fn execute_devql_query(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    events_cfg: &EventsBackendConfig,
    relational: Option<&RelationalStorage>,
) -> Result<Vec<Value>> {
    if (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
        && (parsed.file.is_some() || parsed.files_path.is_some() || parsed.has_artefacts_stage)
    {
        log_devql_validation_failure(
            parsed,
            "telemetry_or_checkpoints_with_artefacts",
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query",
        );
        bail!(
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query"
        )
    }

    if parsed.has_chat_history_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "chat_history_requires_artefacts",
            "chatHistory() requires an artefacts() stage in the query",
        );
        bail!("chatHistory() requires an artefacts() stage in the query");
    }

    if parsed.has_clones_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "clones_requires_artefacts",
            "clones() requires an artefacts() stage in the query",
        );
        bail!("clones() requires an artefacts() stage in the query");
    }

    if parsed.has_deps_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "deps_with_chat_history",
            "deps() cannot be combined with chatHistory() stage",
        );
        bail!("deps() cannot be combined with chatHistory() stage");
    }

    if parsed.has_clones_stage && parsed.has_deps_stage {
        log_devql_validation_failure(
            parsed,
            "clones_with_deps",
            "clones() cannot be combined with deps() stage",
        );
        bail!("clones() cannot be combined with deps() stage");
    }

    if parsed.has_chat_history_stage && (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
    {
        log_devql_validation_failure(
            parsed,
            "chat_history_with_telemetry_or_checkpoints",
            "chatHistory() cannot be combined with checkpoints()/telemetry() stages",
        );
        bail!("chatHistory() cannot be combined with checkpoints()/telemetry() stages");
    }

    if parsed.has_clones_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "clones_with_chat_history",
            "clones() cannot be combined with chatHistory() stage",
        );
        bail!("clones() cannot be combined with chatHistory() stage");
    }

    if parsed.has_clones_stage && parsed.as_of.is_some() {
        log_devql_validation_failure(
            parsed,
            "clones_with_asof",
            "clones() does not yet support asOf(...) queries",
        );
        bail!("clones() does not yet support asOf(...) queries");
    }

    let has_tests_stage = has_registered_tests_stage(parsed);
    let has_coverage_stage = has_registered_coverage_stage(parsed);

    if has_tests_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "tests_requires_artefacts",
            "tests() requires an artefacts() stage in the query",
        );
        bail!("tests() requires an artefacts() stage in the query");
    }

    if has_tests_stage && parsed.has_deps_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_deps",
            "tests() cannot be combined with deps() stage",
        );
        bail!("tests() cannot be combined with deps() stage");
    }

    if has_tests_stage && parsed.has_clones_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_clones",
            "tests() cannot be combined with clones() stage",
        );
        bail!("tests() cannot be combined with clones() stage");
    }

    if has_tests_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_chat_history",
            "tests() cannot be combined with chatHistory() stage",
        );
        bail!("tests() cannot be combined with chatHistory() stage");
    }

    if has_coverage_stage && has_tests_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_tests",
            "coverage() cannot be combined with tests() stage",
        );
        bail!("coverage() cannot be combined with tests() stage");
    }

    if has_tests_stage && has_non_tests_or_coverage_registered_stage(parsed) {
        log_devql_validation_failure(
            parsed,
            "tests_with_non_test_harness_stage",
            "tests() cannot currently be combined with additional registered capability-pack stages",
        );
        bail!(
            "tests() cannot currently be combined with additional registered capability-pack stages"
        );
    }

    if has_coverage_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_requires_artefacts",
            "coverage() requires an artefacts() stage in the query",
        );
        bail!("coverage() requires an artefacts() stage in the query");
    }

    if has_coverage_stage && parsed.has_deps_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_deps",
            "coverage() cannot be combined with deps() stage",
        );
        bail!("coverage() cannot be combined with deps() stage");
    }

    if has_coverage_stage && parsed.has_clones_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_clones",
            "coverage() cannot be combined with clones() stage",
        );
        bail!("coverage() cannot be combined with clones() stage");
    }

    if has_coverage_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_chat_history",
            "coverage() cannot be combined with chatHistory() stage",
        );
        bail!("coverage() cannot be combined with chatHistory() stage");
    }

    if has_coverage_stage && has_non_tests_or_coverage_registered_stage(parsed) {
        log_devql_validation_failure(
            parsed,
            "coverage_with_non_test_harness_stage",
            "coverage() cannot currently be combined with additional registered capability-pack stages",
        );
        bail!(
            "coverage() cannot currently be combined with additional registered capability-pack stages"
        );
    }

    if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        return if events_cfg.has_clickhouse() {
            execute_clickhouse_pipeline(cfg, parsed).await
        } else {
            execute_duckdb_pipeline(cfg, events_cfg, parsed).await
        };
    }

    let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
    execute_relational_pipeline(cfg, events_cfg, parsed, relational).await
}

pub(crate) fn log_devql_validation_failure(parsed: &ParsedDevqlQuery, rule: &str, reason: &str) {
    let has_tests_stage = has_registered_tests_stage(parsed);
    let has_coverage_stage = has_registered_coverage_stage(parsed);
    crate::telemetry::logging::warn(
        &crate::telemetry::logging::with_component(
            crate::telemetry::logging::background(),
            "devql",
        ),
        "devql query validation failed",
        &[
            crate::telemetry::logging::string_attr("rule", rule),
            crate::telemetry::logging::string_attr("reason", reason),
            crate::telemetry::logging::bool_attr("has_deps_stage", parsed.has_deps_stage),
            crate::telemetry::logging::bool_attr("has_clones_stage", parsed.has_clones_stage),
            crate::telemetry::logging::bool_attr(
                "has_chat_history_stage",
                parsed.has_chat_history_stage,
            ),
            crate::telemetry::logging::bool_attr(
                "has_checkpoints_stage",
                parsed.has_checkpoints_stage,
            ),
            crate::telemetry::logging::bool_attr("has_telemetry_stage", parsed.has_telemetry_stage),
            crate::telemetry::logging::bool_attr("has_tests_stage", has_tests_stage),
            crate::telemetry::logging::bool_attr("has_coverage_stage", has_coverage_stage),
            crate::telemetry::logging::bool_attr(
                "has_registered_stages",
                !parsed.registered_stages.is_empty(),
            ),
        ],
    );
}

pub(crate) fn has_registered_tests_stage(parsed: &ParsedDevqlQuery) -> bool {
    parsed
        .registered_stages
        .iter()
        .any(|stage| is_tests_stage_name(&stage.stage_name))
}

pub(crate) fn has_registered_coverage_stage(parsed: &ParsedDevqlQuery) -> bool {
    parsed
        .registered_stages
        .iter()
        .any(|stage| is_coverage_stage_name(&stage.stage_name))
}

pub(crate) fn has_non_tests_or_coverage_registered_stage(parsed: &ParsedDevqlQuery) -> bool {
    parsed.registered_stages.iter().any(|stage| {
        !is_tests_stage_name(&stage.stage_name) && !is_coverage_stage_name(&stage.stage_name)
    })
}

pub(crate) fn is_tests_stage_name(stage_name: &str) -> bool {
    stage_name == crate::capability_packs::test_harness::types::TEST_HARNESS_TESTS_STAGE_ID
}

pub(crate) fn is_coverage_stage_name(stage_name: &str) -> bool {
    stage_name == crate::capability_packs::test_harness::types::TEST_HARNESS_COVERAGE_STAGE_ID
}
