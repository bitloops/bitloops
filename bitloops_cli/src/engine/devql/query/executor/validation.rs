async fn execute_devql_query(
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

    if parsed.has_tests_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "tests_requires_artefacts",
            "tests() requires an artefacts() stage in the query",
        );
        bail!("tests() requires an artefacts() stage in the query");
    }

    if parsed.has_tests_stage && parsed.has_deps_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_deps",
            "tests() cannot be combined with deps() stage",
        );
        bail!("tests() cannot be combined with deps() stage");
    }

    if parsed.has_tests_stage && parsed.has_clones_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_clones",
            "tests() cannot be combined with clones() stage",
        );
        bail!("tests() cannot be combined with clones() stage");
    }

    if parsed.has_tests_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "tests_with_chat_history",
            "tests() cannot be combined with chatHistory() stage",
        );
        bail!("tests() cannot be combined with chatHistory() stage");
    }

    if parsed.has_tests_stage && !parsed.registered_stages.is_empty() {
        log_devql_validation_failure(
            parsed,
            "tests_with_registered_stage",
            "tests() cannot be combined with registered capability-pack stages while built-in tests() remains the active execution path",
        );
        bail!(
            "tests() cannot be combined with registered capability-pack stages while built-in tests() remains the active execution path"
        );
    }

    if parsed.has_coverage_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_requires_artefacts",
            "coverage() requires an artefacts() stage in the query",
        );
        bail!("coverage() requires an artefacts() stage in the query");
    }

    if parsed.has_coverage_stage && parsed.has_deps_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_deps",
            "coverage() cannot be combined with deps() stage",
        );
        bail!("coverage() cannot be combined with deps() stage");
    }

    if parsed.has_coverage_stage && parsed.has_clones_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_clones",
            "coverage() cannot be combined with clones() stage",
        );
        bail!("coverage() cannot be combined with clones() stage");
    }

    if parsed.has_coverage_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_chat_history",
            "coverage() cannot be combined with chatHistory() stage",
        );
        bail!("coverage() cannot be combined with chatHistory() stage");
    }

    if parsed.has_coverage_stage && !parsed.registered_stages.is_empty() {
        log_devql_validation_failure(
            parsed,
            "coverage_with_registered_stage",
            "coverage() cannot be combined with registered capability-pack stages while built-in coverage() remains the active execution path",
        );
        bail!(
            "coverage() cannot be combined with registered capability-pack stages while built-in coverage() remains the active execution path"
        );
    }

    if parsed.has_coverage_stage && parsed.has_tests_stage {
        log_devql_validation_failure(
            parsed,
            "coverage_with_tests",
            "coverage() cannot be combined with tests() stage",
        );
        bail!("coverage() cannot be combined with tests() stage");
    }

    if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        return match events_cfg.provider {
            EventsProvider::ClickHouse => execute_clickhouse_pipeline(cfg, parsed).await,
            EventsProvider::DuckDb => execute_duckdb_pipeline(cfg, events_cfg, parsed).await,
        };
    }

    let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
    execute_relational_pipeline(cfg, events_cfg, parsed, relational).await
}

fn log_devql_validation_failure(parsed: &ParsedDevqlQuery, rule: &str, reason: &str) {
    crate::engine::logging::warn(
        &crate::engine::logging::with_component(crate::engine::logging::background(), "devql"),
        "devql query validation failed",
        &[
            crate::engine::logging::string_attr("rule", rule),
            crate::engine::logging::string_attr("reason", reason),
            crate::engine::logging::bool_attr("has_deps_stage", parsed.has_deps_stage),
            crate::engine::logging::bool_attr("has_clones_stage", parsed.has_clones_stage),
            crate::engine::logging::bool_attr(
                "has_chat_history_stage",
                parsed.has_chat_history_stage,
            ),
            crate::engine::logging::bool_attr(
                "has_checkpoints_stage",
                parsed.has_checkpoints_stage,
            ),
            crate::engine::logging::bool_attr("has_telemetry_stage", parsed.has_telemetry_stage),
            crate::engine::logging::bool_attr("has_tests_stage", parsed.has_tests_stage),
            crate::engine::logging::bool_attr("has_coverage_stage", parsed.has_coverage_stage),
            crate::engine::logging::bool_attr(
                "has_registered_stages",
                !parsed.registered_stages.is_empty(),
            ),
        ],
    );
}
