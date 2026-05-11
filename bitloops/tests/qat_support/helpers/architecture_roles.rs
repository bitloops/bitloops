const ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT: &str = "architecture_graph";
const ARCHITECTURE_GRAPH_SNAPSHOT_MAILBOX_QAT: &str = "architecture_graph.snapshot";
const ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_QAT: &str =
    "architecture_graph.role_adjudication";
const ARCHITECTURE_ROLE_ADJUDICATION_MAILBOX_QAT: &str =
    "architecture_graph.roles.adjudication";

pub fn create_architecture_role_intelligence_fixture_modules(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let crate_dir = world.repo_dir().join("crates/bitloops-inference");
    let src_dir = crate_dir.join("src");
    fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;

    fs::write(
        src_dir.join("storage.rs"),
        "use std::collections::BTreeMap;\n\n#[derive(Default)]\npub struct ProfileStore {\n    values: BTreeMap<String, String>,\n}\n\nimpl ProfileStore {\n    pub fn save_profile(&mut self, key: String, value: String) {\n        self.values.insert(key, value);\n    }\n\n    pub fn load_profile(&self, key: &str) -> Option<&String> {\n        self.values.get(key)\n    }\n}\n",
    )
    .context("writing architecture role storage fixture")?;
    fs::write(
        src_dir.join("current_state.rs"),
        "pub struct RuntimeStateConsumer;\n\nimpl RuntimeStateConsumer {\n    pub fn consume_current_state(&self, changed_path: &str) -> bool {\n        changed_path.ends_with(\".rs\")\n    }\n}\n",
    )
    .context("writing architecture role current-state fixture")?;
    fs::write(
        src_dir.join("register.rs"),
        "pub struct CapabilityRegistrar;\n\nimpl CapabilityRegistrar {\n    pub fn register_bitloops_inference_capability(&self, capability_id: &str) -> String {\n        format!(\"registered:{capability_id}\")\n    }\n}\n",
    )
    .context("writing architecture role capability registration fixture")?;

    let lib_path = src_dir.join("lib.rs");
    let mut lib = fs::read_to_string(&lib_path)
        .with_context(|| format!("reading {}", lib_path.display()))?;
    for module in ["current_state", "register", "storage"] {
        let declaration = format!("mod {module};");
        if !lib.lines().any(|line| line.trim() == declaration) {
            lib = format!("{declaration}\n{lib}");
        }
    }
    fs::write(&lib_path, lib).with_context(|| format!("writing {}", lib_path.display()))?;
    ensure_gitignore_contains(world.repo_dir(), "target/")?;

    append_world_log(
        world,
        "Created architecture role intelligence fixture modules for bitloops-inference.\n",
    )?;
    Ok(())
}

pub fn configure_deterministic_architecture_role_inference(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
        .context("resolving repo id for deterministic architecture role inference")?;
    let provider_role_id =
        bitloops::capability_packs::architecture_graph::roles::storage::deterministic_role_id(
            &repo_id,
            "provider_adapter",
        );
    let seed_payload = deterministic_architecture_role_seed_payload();
    let adjudication_payload = deterministic_architecture_role_adjudication_payload(
        &provider_role_id,
    );
    let (seed_command, seed_args, seed_script_path) =
        fake_architecture_structured_runtime_command_and_args(
            world,
            "architecture-role-seed",
            "qat_architecture_roles_seed",
            "qat-architecture-seed-model",
            &seed_payload,
        )?;
    let (adjudication_command, adjudication_args, adjudication_script_path) =
        fake_architecture_structured_runtime_command_and_args(
            world,
            "architecture-role-adjudication",
            "qat_architecture_roles_adjudication",
            "qat-architecture-adjudication-model",
            &adjudication_payload,
        )?;
    let config = render_architecture_role_inference_config(
        &seed_command,
        &seed_args,
        &adjudication_command,
        &adjudication_args,
    );
    append_scenario_capability_config(world, &config)?;
    append_world_log(
        world,
        &format!(
            "Configured deterministic architecture role inference runtimes at {} and {}.\n",
            seed_script_path.display(),
            adjudication_script_path.display()
        ),
    )?;
    Ok(())
}

pub fn run_architecture_role_seed(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &["devql", "architecture", "roles", "seed"],
        "bitloops devql architecture roles seed",
    )
    .map(|_| ())
}

pub fn run_architecture_roles_bootstrap(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &["devql", "architecture", "roles", "bootstrap", "--json"],
        "bitloops devql architecture roles bootstrap --json",
    )
    .map(|_| ())
}

pub fn activate_seeded_architecture_role_rules(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT rule.rule_id
             FROM architecture_role_detection_rules rule
             JOIN architecture_roles role
               ON role.repo_id = rule.repo_id AND role.role_id = rule.role_id
             WHERE rule.repo_id = ?1
               AND rule.lifecycle_status = 'draft'
             ORDER BY role.canonical_key ASC, rule.version ASC",
        )
        .context("preparing seeded architecture role rule query")?;
    let rules = stmt
        .query_map([repo_id.as_str()], |row| row.get::<_, String>(0))
        .context("querying seeded architecture role rules")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting seeded architecture role rule ids")?;
    ensure!(
        !rules.is_empty(),
        "expected seeded architecture role rules to activate"
    );
    drop(stmt);
    drop(conn);

    for rule_id in rules {
        let rule_ref = format!("rule:{rule_id}");
        let output = run_architecture_role_command(
            world,
            &[
                "devql",
                "architecture",
                "roles",
                "rules",
                "activate",
                &rule_ref,
            ],
            "bitloops devql architecture roles rules activate",
        )?;
        let proposal_id = parse_architecture_role_proposal_id(&output)?;
        run_architecture_role_command(
            world,
            &[
                "devql",
                "architecture",
                "roles",
                "proposal",
                "apply",
                &proposal_id,
            ],
            "bitloops devql architecture roles proposal apply",
        )?;
    }

    assert_no_draft_seeded_architecture_role_rules(world)
}

pub fn run_architecture_role_classification_full_refresh(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "classify",
            "--full",
            "--json",
        ],
        "bitloops devql architecture roles classify --full --json",
    )
    .map(|_| ())
}

pub fn seeded_active_architecture_role_rules_classified(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_architecture_roles_bootstrap(world, repo_name)
}

pub fn snapshot_architecture_role_id(
    world: &mut QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_id = architecture_role_id_for_key(world, role_key)?;
    world
        .architecture_role_id_snapshots
        .insert(role_key.to_string(), role_id);
    Ok(())
}

pub fn snapshot_architecture_role_assignment_id(
    world: &mut QatWorld,
    role_key: &str,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let assignment = architecture_role_assignment_for_path(world, role_key, path)?;
    world.architecture_role_assignment_id_snapshots.insert(
        architecture_role_assignment_snapshot_key(role_key, path),
        assignment.assignment_id,
    );
    Ok(())
}

pub fn rename_architecture_role_and_apply_proposal(
    world: &mut QatWorld,
    role_key: &str,
    display_name: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_ref = format!("role:{role_key}");
    let output = run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "rename",
            &role_ref,
            "--display-name",
            display_name,
        ],
        "bitloops devql architecture roles rename",
    )?;
    let proposal_id = parse_architecture_role_proposal_id(&output)?;
    apply_architecture_role_proposal(world, &proposal_id)
}

pub fn rename_architecture_role_and_show_proposal(
    world: &mut QatWorld,
    role_key: &str,
    display_name: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_ref = format!("role:{role_key}");
    let output = run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "rename",
            &role_ref,
            "--display-name",
            display_name,
        ],
        "bitloops devql architecture roles rename",
    )?;
    let proposal_id = parse_architecture_role_proposal_id(&output)?;
    world.last_architecture_role_proposal_id = Some(proposal_id);
    show_latest_architecture_role_proposal(world, repo_name)
}

pub fn deprecate_architecture_role_without_replacement_and_apply_proposal(
    world: &mut QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_ref = format!("role:{role_key}");
    let output = run_architecture_role_command(
        world,
        &["devql", "architecture", "roles", "deprecate", &role_ref],
        "bitloops devql architecture roles deprecate",
    )?;
    let proposal_id = parse_architecture_role_proposal_id(&output)?;
    apply_architecture_role_proposal(world, &proposal_id)
}

pub fn deprecate_architecture_role_without_replacement_and_show_proposal(
    world: &mut QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_ref = format!("role:{role_key}");
    let output = run_architecture_role_command(
        world,
        &["devql", "architecture", "roles", "deprecate", &role_ref],
        "bitloops devql architecture roles deprecate",
    )?;
    let proposal_id = parse_architecture_role_proposal_id(&output)?;
    world.last_architecture_role_proposal_id = Some(proposal_id);
    show_latest_architecture_role_proposal(world, repo_name)
}

pub fn show_latest_architecture_role_proposal(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let proposal_id = latest_architecture_role_proposal_id(world)?;
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "proposal",
            "show",
            &proposal_id,
        ],
        "bitloops devql architecture roles proposal show",
    )
    .map(|_| ())
}

pub fn apply_latest_architecture_role_proposal(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let proposal_id = latest_architecture_role_proposal_id(world)?;
    apply_architecture_role_proposal(world, &proposal_id)
}

pub fn snapshot_architecture_role_assignments_for_role(
    world: &mut QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let assignments = architecture_role_assignments_for_role(world, role_key, None)?;
    world
        .architecture_role_assignment_set_snapshots
        .insert(role_key.to_string(), assignments);
    Ok(())
}

pub fn snapshot_architecture_role_fact_generation(
    world: &mut QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let generation = architecture_role_fact_generation_for_path(world, path)?;
    world
        .architecture_role_fact_generation_snapshots
        .insert(path.to_string(), generation);
    Ok(())
}

pub fn snapshot_architecture_role_assignment_ids_except_path(
    world: &mut QatWorld,
    excluded_path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let assignments = all_architecture_role_assignments(world, Some(excluded_path))?;
    world.architecture_role_assignment_set_snapshots.insert(
        architecture_role_assignment_exclusion_snapshot_key(excluded_path),
        assignments,
    );
    Ok(())
}

pub fn preview_architecture_role_rule_edit(
    world: &mut QatWorld,
    role_key: &str,
    _removed_path: &str,
    added_path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let rule_id = active_architecture_role_rule_id(world, role_key)?;
    let spec_path = world
        .run_dir()
        .join("architecture-role-rule-edit")
        .join(format!("{role_key}.json"));
    ensure_parent_dir(&spec_path)?;
    let spec = serde_json::json!({
        "role_ref": format!("role:{role_key}"),
        "candidate_selector": {
            "path_prefixes": [],
            "path_suffixes": [added_path],
            "path_contains": [],
            "languages": [],
            "canonical_kinds": [],
            "symbol_fqn_contains": []
        },
        "positive_conditions": [{
            "kind": "path_suffix",
            "value": added_path
        }],
        "negative_conditions": [],
        "score": {
            "base_confidence": 1.0,
            "weight": 1.0
        },
        "evidence": [{"source": "qat_rule_edit_preview"}],
        "metadata": {"source": "qat_architecture_roles"}
    });
    fs::write(&spec_path, serde_json::to_string_pretty(&spec)?)
        .with_context(|| format!("writing {}", spec_path.display()))?;
    let rule_ref = format!("rule:{rule_id}");
    let spec_arg = spec_path.display().to_string();
    let output = run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "rules",
            "edit",
            &rule_ref,
            "--spec",
            &spec_arg,
        ],
        "bitloops devql architecture roles rules edit",
    )?;
    let proposal_id = parse_architecture_role_proposal_id(&output)?;
    world.last_architecture_role_proposal_id = Some(proposal_id.clone());
    world.last_architecture_role_rule_edit_proposal_id = Some(proposal_id);
    Ok(())
}

pub fn run_architecture_roles_status_json(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &["devql", "architecture", "roles", "status", "--json"],
        "bitloops devql architecture roles status --json",
    )
    .map(|_| ())
}

pub fn run_architecture_role_classification_paths_json(
    world: &mut QatWorld,
    paths: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "classify",
            "--paths",
            paths,
            "--json",
        ],
        "bitloops devql architecture roles classify --paths --json",
    )
    .map(|_| ())
}

pub fn run_architecture_role_classification_paths_json_with_adjudication_disabled(
    world: &mut QatWorld,
    paths: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "classify",
            "--paths",
            paths,
            "--enqueue-adjudication=false",
            "--json",
        ],
        "bitloops devql architecture roles classify --paths --enqueue-adjudication=false --json",
    )
    .map(|_| ())
}

pub fn run_architecture_role_classification_repair_stale_json(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "classify",
            "--repair-stale",
            "--json",
        ],
        "bitloops devql architecture roles classify --repair-stale --json",
    )
    .map(|_| ())
}

pub fn create_ambiguous_architecture_role_fixture_path(
    world: &mut QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let absolute = world.repo_dir().join(path);
    ensure_parent_dir(&absolute)?;
    fs::write(
        &absolute,
        "pub trait DynamicProvider {\n    fn invoke(&self, payload: &str) -> serde_json::Value;\n}\n\npub struct JsonDynamicProvider;\n\nimpl DynamicProvider for JsonDynamicProvider {\n    fn invoke(&self, payload: &str) -> serde_json::Value {\n        serde_json::json!({ \"payload\": payload })\n    }\n}\n",
    )
    .with_context(|| format!("writing {}", absolute.display()))?;
    Ok(())
}

pub fn remove_source_file_for_repo(
    world: &mut QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let absolute = world.repo_dir().join(path);
    fs::remove_file(&absolute).with_context(|| format!("removing {}", absolute.display()))
}

pub async fn process_architecture_role_adjudication_job_for_path(
    world: &mut QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let job = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("ArchitectureGraph role adjudication job for `{path}`"),
        || latest_architecture_role_adjudication_job_for_path(world, path),
        |observation| {
            observation
                .as_ref()
                .is_some_and(|job| job.status != "failed" && job.last_error.is_none())
        },
        |observation| match observation {
            Some(job) => format!("status={} last_error={:?}", job.status, job.last_error),
            None => "no job".to_string(),
        },
    )?
    .ok_or_else(|| anyhow!("ArchitectureGraph role adjudication job for `{path}` disappeared"))?;
    if job.status == "completed" {
        return Ok(());
    }

    let relational_path = relational_db_path_for_world(world)
        .context("resolving relational store for architecture role adjudication job")?;
    mark_architecture_role_adjudication_job_running(world, &job.job_id)?;
    let result = {
        let _guard = enter_scenario_app_env(world);
        invoke_architecture_role_adjudication_job(world, &job.payload, relational_path).await
    };
    persist_architecture_role_adjudication_job_outcome(world, &job.job_id, result.as_ref().err())?;
    result?;

    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "ArchitectureGraph role adjudication job completion",
        || latest_architecture_role_adjudication_job_status(world, Some(path)),
        |observation| {
            observation
                .as_ref()
                .is_some_and(|job| job.status == "completed" && job.last_error.is_none())
        },
        |observation| match observation {
            Some(job) => format!(
                "path={:?} status={} last_error={:?}",
                job.path, job.status, job.last_error
            ),
            None => "no job".to_string(),
        },
    )
    .map(|_| ())
}

pub fn assert_architecture_roles_include_keys(
    world: &QatWorld,
    keys_csv: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    for key in split_csv(keys_csv) {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM architecture_roles
                 WHERE repo_id = ?1 AND canonical_key = ?2",
                rusqlite::params![repo_id.as_str(), key.as_str()],
                |row| row.get(0),
            )
            .with_context(|| format!("querying architecture role `{key}`"))?;
        ensure!(count == 1, "expected architecture role `{key}` to exist");
    }
    Ok(())
}

pub fn assert_architecture_role_facts_include_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = architecture_role_facts_count_for_path(world, path)?;
    ensure!(
        count > 0,
        "expected architecture role facts for path `{path}`"
    );
    Ok(())
}

pub fn assert_architecture_role_facts_do_not_include_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = architecture_role_facts_count_for_path(world, path)?;
    ensure!(
        count == 0,
        "expected no architecture role facts for path `{path}`, got {count}"
    );
    Ok(())
}

pub fn assert_architecture_role_facts_newer_than_snapshot(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let previous = world
        .architecture_role_fact_generation_snapshots
        .get(path)
        .copied()
        .ok_or_else(|| anyhow!("missing architecture role fact generation snapshot for `{path}`"))?;
    let current = architecture_role_fact_generation_for_path(world, path)?;
    ensure!(
        current > previous,
        "expected architecture role facts for `{path}` to have newer generation than {previous}, got {current}"
    );
    Ok(())
}

pub fn assert_architecture_role_rule_signal_for_path(
    world: &QatWorld,
    role_key: &str,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM architecture_role_rule_signals_current
             WHERE repo_id = ?1
               AND role_id = ?2
               AND path = ?3
               AND polarity = 'positive'",
            rusqlite::params![repo_id.as_str(), role_id.as_str(), path],
            |row| row.get(0),
        )
        .context("querying architecture role rule signals")?;
    ensure!(
        count > 0,
        "expected positive architecture role rule signal for role `{role_key}` and path `{path}`"
    );
    Ok(())
}

pub fn assert_architecture_role_assignment_active_with_source(
    world: &QatWorld,
    role_key: &str,
    path: &str,
    source: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let assignment = architecture_role_assignment_for_path(world, role_key, path)?;
    ensure!(
        assignment.status == "active",
        "expected role `{role_key}` assignment for `{path}` to be active, got `{}`",
        assignment.status
    );
    ensure!(
        assignment.source == source,
        "expected role `{role_key}` assignment for `{path}` to have source `{source}`, got `{}`",
        assignment.source
    );
    Ok(())
}

pub fn assert_architecture_role_assignment_status(
    world: &QatWorld,
    role_key: &str,
    path: &str,
    expected_status: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let assignment = architecture_role_assignment_for_path(world, role_key, path)?;
    ensure!(
        assignment.status == expected_status,
        "expected role `{role_key}` assignment for `{path}` to have status `{expected_status}`, got `{}`",
        assignment.status
    );
    Ok(())
}

pub fn assert_architecture_role_classification_output_wrote_at_least_assignments(
    world: &QatWorld,
    minimum: u64,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let stdout = world
        .last_command_stdout
        .as_deref()
        .ok_or_else(|| anyhow!("missing last command stdout for architecture role classification"))?;
    let output: serde_json::Value = serde_json::from_str(stdout.trim())
        .context("parsing architecture role classification JSON output")?;
    let assignments_written = output
        .get("roles")
        .and_then(|roles| roles.get("assignments_written"))
        .or_else(|| {
            output
                .get("classification")
                .and_then(|classification| classification.get("roles"))
                .and_then(|roles| roles.get("assignments_written"))
        })
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            anyhow!(
                "classification output missing roles.assignments_written or classification.roles.assignments_written: {output}"
            )
        })?;
    ensure!(
        assignments_written >= minimum,
        "expected architecture role classification output to write at least {minimum} role assignments, got {assignments_written}; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_adjudication_queue_has_no_job_for_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let conn = open_scenario_runtime_sqlite(world)?;
    let count = conn
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*)
                 FROM capability_workplane_jobs
                 WHERE capability_id = ?1
                   AND mailbox_name = ?2
                   AND json_extract(payload, '$.request.path') = ?3",
                rusqlite::params![
                    ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
                    ARCHITECTURE_ROLE_ADJUDICATION_MAILBOX_QAT,
                    path
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(anyhow::Error::from)
        })
        .context("querying architecture role adjudication jobs")?;
    ensure!(
        count == 0,
        "expected no architecture role adjudication job for path `{path}`, got {count}"
    );
    Ok(())
}

pub fn assert_architecture_role_adjudication_job_exists_for_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture role adjudication job for `{path}`"),
        || latest_architecture_role_adjudication_job_status(world, Some(path)),
        |observation| {
            observation
                .as_ref()
                .is_some_and(|job| job.status != "failed" && job.last_error.is_none())
        },
        |observation| match observation {
            Some(job) => format!("status={} last_error={:?}", job.status, job.last_error),
            None => "no job".to_string(),
        },
    )
    .map(|_| ())
}

pub fn assert_architecture_role_assignment_includes_llm_evidence(
    world: &QatWorld,
    role_key: &str,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let row = architecture_role_assignment_json_for_path(world, role_key, path)?;
    let evidence: serde_json::Value =
        serde_json::from_str(&row.evidence_json).context("parsing assignment evidence json")?;
    let provenance: serde_json::Value =
        serde_json::from_str(&row.provenance_json).context("parsing assignment provenance json")?;
    ensure!(
        evidence
            .get("reasoningSummary")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| value.contains("provider adapter")),
        "expected LLM reasoning summary in architecture role assignment evidence for `{path}`"
    );
    ensure!(
        provenance
            .get("source")
            .and_then(serde_json::Value::as_str)
            == Some("llm"),
        "expected LLM provenance source for architecture role assignment `{path}`"
    );
    Ok(())
}

pub fn assert_architecture_role_display_name(
    world: &QatWorld,
    role_key: &str,
    display_name: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let actual = architecture_role_field_for_key(world, role_key, "display_name")?;
    ensure!(
        actual == display_name,
        "expected architecture role `{role_key}` display name `{display_name}`, got `{actual}`"
    );
    Ok(())
}

pub fn assert_architecture_role_lifecycle(
    world: &QatWorld,
    role_key: &str,
    lifecycle: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let actual = architecture_role_field_for_key(world, role_key, "lifecycle_status")?;
    ensure!(
        actual == lifecycle,
        "expected architecture role `{role_key}` lifecycle `{lifecycle}`, got `{actual}`"
    );
    Ok(())
}

pub fn assert_architecture_role_id_matches_snapshot(
    world: &QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let expected = world
        .architecture_role_id_snapshots
        .get(role_key)
        .ok_or_else(|| anyhow!("missing architecture role id snapshot for `{role_key}`"))?;
    let actual = architecture_role_id_for_key(world, role_key)?;
    ensure!(
        &actual == expected,
        "expected architecture role `{role_key}` id to remain `{expected}`, got `{actual}`"
    );
    Ok(())
}

pub fn assert_architecture_role_assignment_id_matches_snapshot(
    world: &QatWorld,
    role_key: &str,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let key = architecture_role_assignment_snapshot_key(role_key, path);
    let expected = world
        .architecture_role_assignment_id_snapshots
        .get(&key)
        .ok_or_else(|| anyhow!("missing architecture role assignment snapshot for `{key}`"))?;
    let actual = architecture_role_assignment_for_path(world, role_key, path)?.assignment_id;
    ensure!(
        &actual == expected,
        "expected architecture role assignment `{key}` id to remain `{expected}`, got `{actual}`"
    );
    Ok(())
}

pub fn assert_architecture_role_rule_edit_preview_removed_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    assert_architecture_role_rule_edit_preview_contains_path(world, path, "removed_matches")
}

pub fn assert_architecture_role_rule_edit_preview_added_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    assert_architecture_role_rule_edit_preview_contains_path(world, path, "added_matches")
}

pub fn assert_architecture_role_assignments_for_role_match_snapshot(
    world: &QatWorld,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let expected = world
        .architecture_role_assignment_set_snapshots
        .get(role_key)
        .ok_or_else(|| anyhow!("missing architecture role assignment snapshot for `{role_key}`"))?;
    let actual = architecture_role_assignments_for_role(world, role_key, None)?;
    ensure!(
        &actual == expected,
        "expected architecture role assignments for `{role_key}` to remain unchanged\nexpected={expected:?}\nactual={actual:?}"
    );
    Ok(())
}

pub fn assert_architecture_role_assignment_ids_except_path_match_snapshot(
    world: &QatWorld,
    excluded_path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let key = architecture_role_assignment_exclusion_snapshot_key(excluded_path);
    let expected = world
        .architecture_role_assignment_set_snapshots
        .get(&key)
        .ok_or_else(|| anyhow!("missing architecture role assignment exclusion snapshot `{key}`"))?;
    let actual = all_architecture_role_assignments(world, Some(excluded_path))?;
    ensure!(
        &actual == expected,
        "expected architecture role assignments except `{excluded_path}` to remain unchanged\nexpected={expected:?}\nactual={actual:?}"
    );
    Ok(())
}

pub fn assert_architecture_graph_sync_handler_completed(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "ArchitectureGraph current-state sync handler completion",
        || latest_architecture_graph_cursor_run(world),
        |observation| {
            observation
                .as_ref()
                .is_some_and(|run| run.status == "completed" && run.error.is_none())
        },
        |observation| match observation {
            Some(run) => format!(
                "status={} from={} to={} mode={} error={:?}",
                run.status, run.from_generation, run.to_generation, run.reconcile_mode, run.error
            ),
            None => "no cursor run".to_string(),
        },
    )
    .map(|_| ())
}

pub fn assert_latest_architecture_role_metrics_full_reconcile(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let metrics = latest_architecture_graph_role_metrics(world)?;
    ensure!(
        metrics
            .get("full_reconcile")
            .and_then(serde_json::Value::as_bool)
            == Some(true),
        "expected latest ArchitectureGraph role metrics full_reconcile=true, got {metrics}"
    );
    Ok(())
}

pub fn assert_latest_architecture_role_metrics_refreshed_at_least_one_path(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let metrics = latest_architecture_graph_role_metrics(world)?;
    let refreshed = metrics
        .get("refreshed_paths")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    ensure!(
        refreshed >= 1,
        "expected latest ArchitectureGraph role metrics refreshed_paths >= 1, got {metrics}"
    );
    Ok(())
}

pub fn assert_architecture_role_assignment_history_status(
    world: &QatWorld,
    status: &str,
    role_key: &str,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM architecture_role_assignment_history
             WHERE repo_id = ?1
               AND role_id = ?2
               AND path = ?3
               AND new_status = ?4",
            rusqlite::params![repo_id.as_str(), role_id.as_str(), path, status],
            |row| row.get(0),
        )
        .context("querying architecture role assignment history")?;
    ensure!(
        count > 0,
        "expected architecture role assignment history status `{status}` for role `{role_key}` and path `{path}`"
    );
    Ok(())
}

pub fn assert_architecture_role_proposal_output_includes_text(
    world: &QatWorld,
    text: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let stdout = latest_command_stdout(world, "architecture role proposal output")?;
    ensure!(
        stdout.contains(text),
        "expected architecture role proposal output to include `{text}`\nstdout:\n{stdout}"
    );
    Ok(())
}

pub fn assert_architecture_role_status_json_review_item_status_for_role(
    world: &QatWorld,
    status: &str,
    role_key: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let output = latest_architecture_role_json_output(world, "architecture roles status JSON")?;
    let review_items = output
        .get("review_items")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("status JSON missing review_items array: {output}"))?;
    ensure!(
        review_items.iter().any(|item| {
            item.get("role_id").and_then(serde_json::Value::as_str) == Some(role_id.as_str())
                && item.get("status").and_then(serde_json::Value::as_str) == Some(status)
        }),
        "expected status JSON review_items to include role `{role_key}` ({role_id}) with status `{status}`; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_status_json_queue_item_for_path(
    world: &QatWorld,
    path: &str,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = latest_architecture_role_json_output(world, "architecture roles status JSON")?;
    let queue_items = output
        .get("queue_items")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("status JSON missing queue_items array: {output}"))?;
    ensure!(
        queue_items
            .iter()
            .any(|item| item.get("path").and_then(serde_json::Value::as_str) == Some(path)),
        "expected status JSON queue_items to include path `{path}`; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_classification_json_adjudication_candidates_at_least(
    world: &QatWorld,
    minimum: u64,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output =
        latest_architecture_role_json_output(world, "architecture roles classification JSON")?;
    let actual = output
        .get("roles")
        .and_then(|roles| roles.get("adjudication_candidates"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            anyhow!("classification JSON missing roles.adjudication_candidates: {output}")
        })?;
    ensure!(
        actual >= minimum,
        "expected classification JSON roles.adjudication_candidates >= {minimum}, got {actual}; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_classification_json_enqueued_adjudication_jobs(
    world: &QatWorld,
    expected: u64,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output =
        latest_architecture_role_json_output(world, "architecture roles classification JSON")?;
    let actual = output
        .get("role_adjudication_enqueued")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("classification JSON missing role_adjudication_enqueued: {output}"))?;
    ensure!(
        actual == expected,
        "expected classification JSON role_adjudication_enqueued={expected}, got {actual}; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_classification_json_full_reconcile(
    world: &QatWorld,
    expected: bool,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output =
        latest_architecture_role_json_output(world, "architecture roles classification JSON")?;
    let actual = output
        .get("roles")
        .and_then(|roles| roles.get("full_reconcile"))
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| anyhow!("classification JSON missing roles.full_reconcile: {output}"))?;
    ensure!(
        actual == expected,
        "expected classification JSON roles.full_reconcile={expected}, got {actual}; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_classification_json_affected_path_count(
    world: &QatWorld,
    expected: u64,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output =
        latest_architecture_role_json_output(world, "architecture roles classification JSON")?;
    let actual = output
        .get("roles")
        .and_then(|roles| roles.get("affected_paths"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("classification JSON missing roles.affected_paths: {output}"))?;
    ensure!(
        actual == expected,
        "expected classification JSON roles.affected_paths={expected}, got {actual}; output={output}"
    );
    Ok(())
}

pub fn assert_architecture_role_classification_json_includes_stale_assignment_metric(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output =
        latest_architecture_role_json_output(world, "architecture roles classification JSON")?;
    output
        .get("roles")
        .and_then(|roles| roles.get("assignments_marked_stale"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            anyhow!("classification JSON missing roles.assignments_marked_stale: {output}")
        })?;
    Ok(())
}

fn run_architecture_role_command(
    world: &mut QatWorld,
    args: &[&str],
    label: &str,
) -> Result<String> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    ensure_success(&output, label)?;
    Ok(stdout)
}

fn apply_architecture_role_proposal(world: &mut QatWorld, proposal_id: &str) -> Result<()> {
    run_architecture_role_command(
        world,
        &[
            "devql",
            "architecture",
            "roles",
            "proposal",
            "apply",
            proposal_id,
        ],
        "bitloops devql architecture roles proposal apply",
    )
    .map(|_| ())
}

fn latest_architecture_role_proposal_id(world: &QatWorld) -> Result<String> {
    world
        .last_architecture_role_proposal_id
        .clone()
        .ok_or_else(|| anyhow!("missing latest architecture role proposal id"))
}

fn latest_command_stdout<'a>(world: &'a QatWorld, label: &str) -> Result<&'a str> {
    world
        .last_command_stdout
        .as_deref()
        .ok_or_else(|| anyhow!("missing last command stdout for {label}"))
}

fn latest_architecture_role_json_output(world: &QatWorld, label: &str) -> Result<serde_json::Value> {
    let stdout = latest_command_stdout(world, label)?;
    serde_json::from_str(stdout.trim()).with_context(|| format!("parsing {label}:\n{stdout}"))
}

fn parse_architecture_role_proposal_id(stdout: &str) -> Result<String> {
    stdout
        .lines()
        .find_map(|line| {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix("proposal="))
        })
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("missing architecture role proposal id in stdout:\n{stdout}"))
}

fn assert_no_draft_seeded_architecture_role_rules(world: &QatWorld) -> Result<()> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let draft_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM architecture_role_detection_rules
             WHERE repo_id = ?1 AND lifecycle_status = 'draft'",
            [repo_id.as_str()],
            |row| row.get(0),
        )
        .context("querying draft architecture role rules")?;
    ensure!(
        draft_count == 0,
        "expected all seeded architecture role rules to be active, got {draft_count} draft rules"
    );
    Ok(())
}

fn deterministic_architecture_role_seed_payload() -> serde_json::Value {
    let roles = [
        (
            "process_entrypoint",
            "Process Entrypoint",
            "Entry process boundary that starts the executable.",
            "entrypoint",
        ),
        (
            "runtime_bootstrapper",
            "Runtime Bootstrapper",
            "Bootstraps the runtime service loop.",
            "entrypoint",
        ),
        (
            "cli_command_grammar",
            "CLI Command Grammar",
            "Defines command-line grammar and argument parsing.",
            "interface",
        ),
        (
            "command_dispatcher",
            "Command Dispatcher",
            "Dispatches parsed commands to runtime behavior.",
            "application",
        ),
        (
            "storage_adapter",
            "Storage Adapter",
            "Persists and loads durable runtime state.",
            "infrastructure",
        ),
        (
            "current_state_consumer",
            "Current State Consumer",
            "Consumes current-state deltas for capability processing.",
            "integration",
        ),
        (
            "capability_registration",
            "Capability Registration",
            "Registers capability handlers with the host.",
            "integration",
        ),
        (
            "provider_adapter",
            "Provider Adapter",
            "Adapts external inference providers.",
            "infrastructure",
        ),
    ]
    .into_iter()
    .map(|(canonical_key, display_name, description, family)| {
        serde_json::json!({
            "canonical_key": canonical_key,
            "display_name": display_name,
            "description": description,
            "family": family,
            "lifecycle_status": "active",
            "provenance": {"source": "qat_architecture_roles_seed"},
            "evidence": [{"source": "qat_fixture"}]
        })
    })
    .collect::<Vec<_>>();

    let rules = [
        (
            "process_entrypoint",
            "crates/bitloops-inference/src/main.rs",
        ),
        (
            "runtime_bootstrapper",
            "crates/bitloops-inference/src/runtime.rs",
        ),
        (
            "cli_command_grammar",
            "crates/bitloops-inference/src/cli.rs",
        ),
        (
            "command_dispatcher",
            "crates/bitloops-inference/src/lib.rs",
        ),
        (
            "storage_adapter",
            "crates/bitloops-inference/src/storage.rs",
        ),
        (
            "current_state_consumer",
            "crates/bitloops-inference/src/current_state.rs",
        ),
        (
            "capability_registration",
            "crates/bitloops-inference/src/register.rs",
        ),
    ]
    .into_iter()
    .map(|(role_key, path_suffix)| deterministic_architecture_role_rule(role_key, path_suffix))
    .collect::<Vec<_>>();

    serde_json::json!({
        "roles": roles,
        "rule_candidates": rules
    })
}

fn deterministic_architecture_role_rule(role_key: &str, path_suffix: &str) -> serde_json::Value {
    serde_json::json!({
        "target_role_key": role_key,
        "candidate_selector": {
            "path_prefixes": [],
            "path_suffixes": [path_suffix],
            "path_contains": [],
            "languages": [],
            "canonical_kinds": [],
            "symbol_fqn_contains": []
        },
        "positive_conditions": [{
            "kind": "path_suffix",
            "value": path_suffix
        }],
        "negative_conditions": [],
        "score": {
            "base_confidence": 1.0,
            "weight": 1.0
        },
        "evidence": [{"source": "qat_architecture_roles_seed", "path": path_suffix}],
        "metadata": {"source": "qat_architecture_roles"}
    })
}

fn ensure_gitignore_contains(repo_root: &Path, entry: &str) -> Result<()> {
    let gitignore_path = repo_root.join(".gitignore");
    let mut contents = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", gitignore_path.display()));
        }
    };
    if !contents.lines().any(|line| line.trim() == entry) {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(entry);
        contents.push('\n');
        fs::write(&gitignore_path, contents)
            .with_context(|| format!("writing {}", gitignore_path.display()))?;
    }
    Ok(())
}

fn deterministic_architecture_role_adjudication_payload(role_id: &str) -> serde_json::Value {
    serde_json::json!({
        "outcome": "assigned",
        "assignments": [{
            "role_id": role_id,
            "primary": true,
            "confidence": 0.91,
            "evidence": {"source": "qat_fake_role_adjudication"}
        }],
        "confidence": 0.91,
        "evidence": {"source": "qat_fake_role_adjudication"},
        "reasoning_summary": "QAT deterministic provider adapter adjudication.",
        "rule_suggestions": []
    })
}

#[cfg(unix)]
fn fake_architecture_structured_runtime_command_and_args(
    world: &QatWorld,
    script_name: &str,
    profile_name: &str,
    model_name: &str,
    payload: &serde_json::Value,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    use std::os::unix::fs::PermissionsExt;

    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join(format!("fake-{script_name}-runtime.sh"));
    let payload_json = serde_json::to_string(payload)?;
    let script = r#"#!/bin/sh
payload=$(cat <<'JSON'
__PAYLOAD_JSON__
JSON
)
while IFS= read -r line; do
  request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"__PROFILE_NAME__","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"__MODEL_NAME__","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}\n' "$request_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s"}\n' "$request_id"
      exit 0
      ;;
    *'"type":"infer"'*)
      printf '{"type":"infer","request_id":"%s","text":"","parsed_json":%s,"provider_name":"qat","model_name":"__MODEL_NAME__"}\n' "$request_id" "$payload"
      ;;
  esac
done
"#
    .replace("__PAYLOAD_JSON__", &payload_json)
    .replace("__PROFILE_NAME__", profile_name)
    .replace("__MODEL_NAME__", model_name);

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    let mut permissions = fs::metadata(&script_path)
        .with_context(|| format!("reading {}", script_path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions)
        .with_context(|| format!("chmod {}", script_path.display()))?;
    Ok((
        "sh".to_string(),
        vec![script_path.display().to_string()],
        script_path,
    ))
}

#[cfg(windows)]
fn fake_architecture_structured_runtime_command_and_args(
    world: &QatWorld,
    script_name: &str,
    profile_name: &str,
    model_name: &str,
    payload: &serde_json::Value,
) -> Result<(String, Vec<String>, std::path::PathBuf)> {
    let script_path = world
        .run_dir()
        .join("capability-runtime")
        .join(format!("fake-{script_name}-runtime.ps1"));
    let payload_json = serde_json::to_string(payload)?;
    let script = r#"
$payload = @'
__PAYLOAD_JSON__
'@
while (($line = [Console]::In.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $requestId = [regex]::Match($line, '"request_id":"([^"]+)"').Groups[1].Value
  if ($line -like '*"type":"describe"*') {
    Write-Output '{"type":"describe","request_id":"'"$requestId"'","protocol_version":1,"runtime_name":"bitloops-inference","runtime_version":"0.1.0","profile_name":"__PROFILE_NAME__","provider":{"kind":"ollama_chat","provider_name":"qat","model_name":"__MODEL_NAME__","endpoint":"http://127.0.0.1:11434","capabilities":["text","json_object"]}}'
  } elseif ($line -like '*"type":"shutdown"*') {
    Write-Output '{"type":"shutdown","request_id":"'"$requestId"'"}'
    exit 0
  } elseif ($line -like '*"type":"infer"*') {
    Write-Output '{"type":"infer","request_id":"'"$requestId"'","text":"","parsed_json":'$payload',"provider_name":"qat","model_name":"__MODEL_NAME__"}'
  }
}
"#
    .replace("__PAYLOAD_JSON__", &payload_json)
    .replace("__PROFILE_NAME__", profile_name)
    .replace("__MODEL_NAME__", model_name);

    ensure_parent_dir(&script_path)?;
    fs::write(&script_path, script)
        .with_context(|| format!("writing {}", script_path.display()))?;
    Ok((
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
        script_path,
    ))
}

fn render_architecture_role_inference_config(
    seed_command: &str,
    seed_args: &[String],
    adjudication_command: &str,
    adjudication_args: &[String],
) -> String {
    let seed_runtime_args = seed_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let adjudication_runtime_args = adjudication_args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
[architecture.inference]
fact_synthesis = "qat_architecture_roles_seed"
role_adjudication = "qat_architecture_roles_adjudication"

[inference.runtimes.qat_architecture_roles_seed_runtime]
command = {seed_command:?}
args = [{seed_runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.runtimes.qat_architecture_roles_adjudication_runtime]
command = {adjudication_command:?}
args = [{adjudication_runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5

[inference.profiles.qat_architecture_roles_seed]
task = "structured_generation"
driver = "ollama_chat"
runtime = "qat_architecture_roles_seed_runtime"
model = "qat-architecture-seed-model"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 4096

[inference.profiles.qat_architecture_roles_adjudication]
task = "structured_generation"
driver = "ollama_chat"
runtime = "qat_architecture_roles_adjudication_runtime"
model = "qat-architecture-adjudication-model"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 1024
"#,
        seed_command = seed_command,
        seed_runtime_args = seed_runtime_args,
        adjudication_command = adjudication_command,
        adjudication_runtime_args = adjudication_runtime_args,
    )
}

fn architecture_role_id_for_key(world: &QatWorld, role_key: &str) -> Result<String> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    conn.query_row(
        "SELECT role_id
         FROM architecture_roles
         WHERE repo_id = ?1 AND canonical_key = ?2
         LIMIT 1",
        rusqlite::params![repo_id.as_str(), role_key],
        |row| row.get::<_, String>(0),
    )
    .with_context(|| format!("loading architecture role id for `{role_key}`"))
}

fn architecture_role_field_for_key(world: &QatWorld, role_key: &str, field: &str) -> Result<String> {
    ensure!(
        matches!(field, "display_name" | "lifecycle_status"),
        "unsupported architecture role field `{field}`"
    );
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let sql = format!(
        "SELECT {field}
         FROM architecture_roles
         WHERE repo_id = ?1 AND canonical_key = ?2
         LIMIT 1"
    );
    conn.query_row(&sql, rusqlite::params![repo_id.as_str(), role_key], |row| {
        row.get::<_, String>(0)
    })
    .with_context(|| format!("loading architecture role `{role_key}` field `{field}`"))
}

fn active_architecture_role_rule_id(world: &QatWorld, role_key: &str) -> Result<String> {
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    conn.query_row(
        "SELECT rule_id
         FROM architecture_role_detection_rules
         WHERE repo_id = ?1
           AND role_id = ?2
           AND lifecycle_status = 'active'
         ORDER BY version DESC
         LIMIT 1",
        rusqlite::params![repo_id.as_str(), role_id.as_str()],
        |row| row.get::<_, String>(0),
    )
    .with_context(|| format!("loading active architecture role rule for `{role_key}`"))
}

#[derive(Debug, Clone)]
struct ArchitectureRoleAssignmentRow {
    path: String,
    role: String,
    assignment_id: String,
    status: String,
    source: String,
    evidence_json: String,
    provenance_json: String,
}

fn architecture_role_assignment_for_path(
    world: &QatWorld,
    role_key: &str,
    path: &str,
) -> Result<ArchitectureRoleAssignmentSnapshot> {
    let row = architecture_role_assignment_json_for_path(world, role_key, path)?;
    Ok(ArchitectureRoleAssignmentSnapshot {
        path: row.path,
        role: row.role,
        assignment_id: row.assignment_id,
        status: row.status,
        source: row.source,
    })
}

fn architecture_role_assignment_json_for_path(
    world: &QatWorld,
    role_key: &str,
    path: &str,
) -> Result<ArchitectureRoleAssignmentRow> {
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    conn.query_row(
        "SELECT assignment.assignment_id,
                assignment.path,
                role.canonical_key,
                assignment.status,
                assignment.source,
                assignment.evidence_json,
                assignment.provenance_json
         FROM architecture_role_assignments_current assignment
         JOIN architecture_roles role
           ON role.repo_id = assignment.repo_id AND role.role_id = assignment.role_id
         WHERE assignment.repo_id = ?1
           AND assignment.role_id = ?2
           AND assignment.path = ?3
         ORDER BY CASE assignment.status WHEN 'active' THEN 0 WHEN 'needs_review' THEN 1 WHEN 'stale' THEN 2 ELSE 3 END,
                  CASE assignment.priority WHEN 'primary' THEN 0 ELSE 1 END,
                  assignment.assignment_id ASC
         LIMIT 1",
        rusqlite::params![repo_id.as_str(), role_id.as_str(), path],
        |row| {
            Ok(ArchitectureRoleAssignmentRow {
                assignment_id: row.get(0)?,
                path: row.get(1)?,
                role: row.get(2)?,
                status: row.get(3)?,
                source: row.get(4)?,
                evidence_json: row.get(5)?,
                provenance_json: row.get(6)?,
            })
        },
    )
    .with_context(|| {
        format!("loading architecture role assignment for role `{role_key}` and path `{path}`")
    })
}

fn architecture_role_assignments_for_role(
    world: &QatWorld,
    role_key: &str,
    excluded_path: Option<&str>,
) -> Result<Vec<ArchitectureRoleAssignmentSnapshot>> {
    let role_id = architecture_role_id_for_key(world, role_key)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT assignment.path,
                    role.canonical_key,
                    assignment.assignment_id,
                    assignment.status,
                    assignment.source
             FROM architecture_role_assignments_current assignment
             JOIN architecture_roles role
               ON role.repo_id = assignment.repo_id AND role.role_id = assignment.role_id
             WHERE assignment.repo_id = ?1
               AND assignment.role_id = ?2
               AND (?3 IS NULL OR assignment.path != ?3)
             ORDER BY assignment.path ASC, role.canonical_key ASC, assignment.assignment_id ASC",
        )
        .context("preparing architecture role assignments query")?;
    collect_assignment_snapshots(
        stmt.query_map(
            rusqlite::params![repo_id.as_str(), role_id.as_str(), excluded_path],
            assignment_snapshot_from_row,
        )
        .context("querying architecture role assignments")?,
    )
}

fn all_architecture_role_assignments(
    world: &QatWorld,
    excluded_path: Option<&str>,
) -> Result<Vec<ArchitectureRoleAssignmentSnapshot>> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT assignment.path,
                    role.canonical_key,
                    assignment.assignment_id,
                    assignment.status,
                    assignment.source
             FROM architecture_role_assignments_current assignment
             JOIN architecture_roles role
               ON role.repo_id = assignment.repo_id AND role.role_id = assignment.role_id
             WHERE assignment.repo_id = ?1
               AND (?2 IS NULL OR assignment.path != ?2)
               AND assignment.source = 'rule'
             ORDER BY assignment.path ASC, role.canonical_key ASC, assignment.assignment_id ASC",
        )
        .context("preparing all architecture role assignments query")?;
    collect_assignment_snapshots(
        stmt.query_map(
            rusqlite::params![repo_id.as_str(), excluded_path],
            assignment_snapshot_from_row,
        )
        .context("querying all architecture role assignments")?,
    )
}

fn assignment_snapshot_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ArchitectureRoleAssignmentSnapshot> {
    Ok(ArchitectureRoleAssignmentSnapshot {
        path: row.get(0)?,
        role: row.get(1)?,
        assignment_id: row.get(2)?,
        status: row.get(3)?,
        source: row.get(4)?,
    })
}

fn collect_assignment_snapshots<I>(rows: I) -> Result<Vec<ArchitectureRoleAssignmentSnapshot>>
where
    I: Iterator<Item = rusqlite::Result<ArchitectureRoleAssignmentSnapshot>>,
{
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting architecture role assignment snapshots")
}

fn architecture_role_assignment_snapshot_key(role_key: &str, path: &str) -> String {
    format!("{role_key}|{path}")
}

fn architecture_role_assignment_exclusion_snapshot_key(path: &str) -> String {
    format!("except|{path}")
}

fn architecture_role_fact_generation_for_path(world: &QatWorld, path: &str) -> Result<u64> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let generation: Option<i64> = conn
        .query_row(
            "SELECT MAX(generation_seq)
             FROM architecture_artefact_facts_current
             WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id.as_str(), path],
            |row| row.get(0),
        )
        .with_context(|| format!("querying architecture role fact generation for `{path}`"))?;
    generation
        .map(|value| u64::try_from(value).unwrap_or(0))
        .ok_or_else(|| anyhow!("missing architecture role facts for path `{path}`"))
}

fn architecture_role_facts_count_for_path(world: &QatWorld, path: &str) -> Result<i64> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    conn.query_row(
        "SELECT COUNT(*)
         FROM architecture_artefact_facts_current
         WHERE repo_id = ?1 AND path = ?2",
        rusqlite::params![repo_id.as_str(), path],
        |row| row.get(0),
    )
    .with_context(|| format!("querying architecture role facts for `{path}`"))
}

fn assert_architecture_role_rule_edit_preview_contains_path(
    world: &QatWorld,
    path: &str,
    preview_key: &str,
) -> Result<()> {
    let proposal_id = world
        .last_architecture_role_rule_edit_proposal_id
        .as_ref()
        .ok_or_else(|| anyhow!("missing latest architecture role rule edit proposal id"))?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let preview_json: String = conn
        .query_row(
            "SELECT preview_payload_json
             FROM architecture_role_change_proposals
             WHERE repo_id = ?1 AND proposal_id = ?2
             LIMIT 1",
            rusqlite::params![repo_id.as_str(), proposal_id.as_str()],
            |row| row.get(0),
        )
        .context("loading architecture role rule edit preview payload")?;
    let preview: serde_json::Value =
        serde_json::from_str(&preview_json).context("parsing rule edit preview payload")?;
    let matches = preview
        .get(preview_key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("preview payload missing array `{preview_key}`: {preview}"))?;
    let artefact_ids = current_artefact_ids_for_path(&conn, &repo_id, path)?;
    ensure!(
        matches.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|candidate| artefact_ids.iter().any(|id| id == candidate))
        }),
        "expected rule edit preview `{preview_key}` to include a match for path `{path}`; artefact_ids={artefact_ids:?}; preview={preview}"
    );
    Ok(())
}

fn current_artefact_ids_for_path(
    conn: &rusqlite::Connection,
    repo_id: &str,
    path: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT artefact_id
             FROM artefacts_current
             WHERE repo_id = ?1 AND path = ?2
             ORDER BY artefact_id ASC",
        )
        .context("preparing current artefact id query")?;
    let artefact_ids = stmt
        .query_map(rusqlite::params![repo_id, path], |row| {
            row.get::<_, String>(0)
        })
        .context("querying current artefact ids")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("collecting current artefact ids")?;
    ensure!(
        !artefact_ids.is_empty(),
        "expected at least one current artefact for path `{path}`"
    );
    Ok(artefact_ids)
}

#[derive(Debug, Clone)]
struct ArchitectureRoleAdjudicationJobObservation {
    status: String,
    path: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ArchitectureRoleAdjudicationJobForQat {
    job_id: String,
    status: String,
    payload: serde_json::Value,
    last_error: Option<String>,
}

fn latest_architecture_role_adjudication_job_status(
    world: &QatWorld,
    path: Option<&str>,
) -> Result<Option<ArchitectureRoleAdjudicationJobObservation>> {
    let sqlite = open_scenario_runtime_sqlite(world)?;
    sqlite
        .with_connection(|conn| {
            use rusqlite::OptionalExtension as _;

            let map_row = |row: &rusqlite::Row<'_>| {
                Ok(ArchitectureRoleAdjudicationJobObservation {
                    status: row.get(0)?,
                    path: row.get(1)?,
                    last_error: row.get(2)?,
                })
            };
            if let Some(path) = path {
                return conn
                    .query_row(
                        "SELECT status, json_extract(payload, '$.request.path') AS path, last_error
                         FROM capability_workplane_jobs
                         WHERE capability_id = ?1
                           AND mailbox_name = ?2
                           AND json_extract(payload, '$.request.path') = ?3
                         ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC,
                                  submitted_at_unix DESC
                         LIMIT 1",
                        rusqlite::params![
                            ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
                            ARCHITECTURE_ROLE_ADJUDICATION_MAILBOX_QAT,
                            path
                        ],
                        map_row,
                    )
                    .optional()
                    .map_err(anyhow::Error::from);
            }
            conn.query_row(
                "SELECT status, json_extract(payload, '$.request.path') AS path, last_error
                 FROM capability_workplane_jobs
                 WHERE capability_id = ?1
                   AND mailbox_name = ?2
                 ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC,
                          submitted_at_unix DESC
                 LIMIT 1",
                rusqlite::params![
                    ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
                    ARCHITECTURE_ROLE_ADJUDICATION_MAILBOX_QAT
                ],
                map_row,
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .context("loading latest architecture role adjudication job")
}

fn latest_architecture_role_adjudication_job_for_path(
    world: &QatWorld,
    path: &str,
) -> Result<Option<ArchitectureRoleAdjudicationJobForQat>> {
    let sqlite = open_scenario_runtime_sqlite(world)?;
    sqlite
        .with_connection(|conn| {
            use rusqlite::OptionalExtension as _;

            conn.query_row(
                "SELECT job_id, status, payload, last_error
                 FROM capability_workplane_jobs
                 WHERE capability_id = ?1
                   AND mailbox_name = ?2
                   AND json_extract(payload, '$.request.path') = ?3
                 ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC,
                          submitted_at_unix DESC
                 LIMIT 1",
                rusqlite::params![
                    ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
                    ARCHITECTURE_ROLE_ADJUDICATION_MAILBOX_QAT,
                    path
                ],
                |row| {
                    let payload_raw: String = row.get(2)?;
                    let payload = serde_json::from_str(&payload_raw).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?;
                    Ok(ArchitectureRoleAdjudicationJobForQat {
                        job_id: row.get(0)?,
                        status: row.get(1)?,
                        payload,
                        last_error: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .context("loading ArchitectureGraph role adjudication job")
}

fn mark_architecture_role_adjudication_job_running(
    world: &QatWorld,
    job_id: &str,
) -> Result<()> {
    let now = qat_unix_timestamp_i64()?;
    let sqlite = open_scenario_runtime_sqlite(world)?;
    sqlite.with_connection(|conn| {
        conn.execute(
            "UPDATE capability_workplane_jobs
             SET status = 'running',
                 attempts = attempts + 1,
                 started_at_unix = COALESCE(started_at_unix, ?1),
                 updated_at_unix = ?1,
                 lease_owner = 'qat-architecture-roles',
                 lease_expires_at_unix = NULL,
                 last_error = NULL
             WHERE job_id = ?2
               AND status IN ('pending', 'running')",
            rusqlite::params![now, job_id],
        )
        .with_context(|| format!("marking architecture role adjudication job `{job_id}` running"))?;
        Ok(())
    })
}

async fn invoke_architecture_role_adjudication_job(
    world: &QatWorld,
    payload: &serde_json::Value,
    relational_path: std::path::PathBuf,
) -> Result<()> {
    let repo = bitloops::host::devql::resolve_repo_identity(world.repo_dir())
        .context("resolving repo identity for architecture role adjudication job")?;
    let host = bitloops::host::devql::build_capability_host(world.repo_dir(), repo)
        .context("building capability host for architecture role adjudication job")?;
    let relational = bitloops::host::devql::RelationalStorage::local_only(relational_path);
    host.invoke_ingester_with_relational(
        ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_QAT,
        payload.clone(),
        Some(&relational),
    )
    .await
    .context("invoking ArchitectureGraph role adjudication ingester")?;
    Ok(())
}

fn persist_architecture_role_adjudication_job_outcome(
    world: &QatWorld,
    job_id: &str,
    error: Option<&anyhow::Error>,
) -> Result<()> {
    let now = qat_unix_timestamp_i64()?;
    let status = if error.is_some() { "failed" } else { "completed" };
    let last_error = error.map(|err| err.to_string());
    let sqlite = open_scenario_runtime_sqlite(world)?;
    sqlite.with_connection(|conn| {
        conn.execute(
            "UPDATE capability_workplane_jobs
             SET status = ?1,
                 updated_at_unix = ?2,
                 completed_at_unix = ?2,
                 last_error = ?3,
                 lease_owner = NULL,
                 lease_expires_at_unix = NULL
             WHERE job_id = ?4",
            rusqlite::params![status, now, last_error.as_deref(), job_id],
        )
        .with_context(|| {
            format!("persisting architecture role adjudication job `{job_id}` outcome")
        })?;
        Ok(())
    })
}

fn qat_unix_timestamp_i64() -> Result<i64> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    ensure!(now >= 0, "current unix timestamp is negative: {now}");
    Ok(now)
}

#[derive(Debug, Clone)]
struct ArchitectureGraphCursorRunObservation {
    status: String,
    from_generation: i64,
    to_generation: i64,
    reconcile_mode: String,
    error: Option<String>,
}

fn latest_architecture_graph_cursor_run(
    world: &QatWorld,
) -> Result<Option<ArchitectureGraphCursorRunObservation>> {
    let sqlite = open_scenario_runtime_sqlite(world)?;
    sqlite
        .with_connection(|conn| {
            use rusqlite::OptionalExtension as _;
            conn.query_row(
                "SELECT status, from_generation_seq, to_generation_seq, reconcile_mode, error
                 FROM capability_workplane_cursor_runs
                 WHERE capability_id = ?1 AND mailbox_name = ?2
                 ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC,
                          submitted_at_unix DESC
                 LIMIT 1",
                rusqlite::params![
                    ARCHITECTURE_GRAPH_CAPABILITY_ID_QAT,
                    ARCHITECTURE_GRAPH_SNAPSHOT_MAILBOX_QAT
                ],
                |row| {
                    Ok(ArchitectureGraphCursorRunObservation {
                        status: row.get(0)?,
                        from_generation: row.get(1)?,
                        to_generation: row.get(2)?,
                        reconcile_mode: row.get(3)?,
                        error: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .context("loading latest ArchitectureGraph cursor run")
}

fn latest_architecture_graph_role_metrics(world: &QatWorld) -> Result<serde_json::Value> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let metrics_json: String = conn
        .query_row(
            "SELECT metrics_json
             FROM architecture_graph_runs_current
             WHERE repo_id = ?1
             LIMIT 1",
            [repo_id.as_str()],
            |row| row.get(0),
        )
        .context("loading latest ArchitectureGraph metrics")?;
    let metrics: serde_json::Value =
        serde_json::from_str(&metrics_json).context("parsing ArchitectureGraph metrics json")?;
    metrics
        .get("roles")
        .cloned()
        .filter(|value| !value.is_null())
        .ok_or_else(|| anyhow!("latest ArchitectureGraph metrics missing roles section: {metrics}"))
}

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
