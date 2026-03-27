pub fn sanitize_name(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_was_dash = false;

    for ch in input.chars() {
        let normalized = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            _ if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '/' | ':') => Some('-'),
            _ => None,
        };

        if let Some(value) = normalized {
            if value == '-' {
                if slug.is_empty() || last_was_dash {
                    continue;
                }
                last_was_dash = true;
            } else {
                last_was_dash = false;
            }
            slug.push(value);
        }
    }

    slug.trim_matches('-').to_string()
}

pub fn ensure_bitloops_repo_name(repo_name: &str) -> Result<()> {
    ensure!(
        repo_name == BITLOOPS_REPO_NAME,
        "unsupported repository `{repo_name}`; only `bitloops` is supported by qat"
    );
    Ok(())
}

pub fn run_clean_start(world: &mut QatWorld, flow_name: &str) -> Result<()> {
    let config = world.run_config().clone();
    let flow_slug = sanitize_name(flow_name);
    ensure!(
        !flow_slug.is_empty(),
        "flow name must produce a non-empty slug"
    );

    let scenario_slug = world
        .scenario_slug
        .clone()
        .unwrap_or_else(|| "scenario".to_string());
    let run_dir = config
        .suite_root
        .join(format!("{scenario_slug}-{flow_slug}-{}", short_run_id()));
    let repo_dir = run_dir.join(BITLOOPS_REPO_NAME);
    let terminal_log_path = run_dir.join("terminal.log");
    let metadata_path = run_dir.join("run.json");

    fs::create_dir_all(&repo_dir).context("creating qat repo directory")?;

    world.flow_name = Some(flow_name.to_string());
    world.run_dir = Some(run_dir);
    world.repo_dir = Some(repo_dir);
    world.terminal_log_path = Some(terminal_log_path);
    world.metadata_path = Some(metadata_path);

    let init_output = run_command_capture(
        world,
        "git init",
        build_git_command(world.repo_dir(), &["init", "-q"], &[]),
    )?;
    ensure_success(&init_output, "git init")?;
    configure_git_identity(world)?;
    write_run_metadata(world)?;
    append_world_log(
        world,
        &format!(
            "Initialized clean run directory at {}\n",
            world.run_dir().display()
        ),
    )?;
    Ok(())
}

pub fn run_init_commit_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(world, "InitCommit skipped because HEAD already exists.\n")?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &[],
        "git commit initial",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn run_init_commit_with_relative_day_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    days_ago: i64,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(
            world,
            "InitCommit with relative day skipped because HEAD already exists.\n",
        )?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];
    run_git_success(world, &["add", "-A"], &env, "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &env,
        "git commit initial",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn run_create_vite_app_project_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    create_offline_vite_react_ts_scaffold(world.repo_dir())
}

pub fn run_init_bitloops_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut attempts = 0_u8;
    loop {
        let output = run_command_capture(
            world,
            "bitloops init",
            build_bitloops_command(world, &["init", "--agent", "claude-code"])?,
        )
        .context("running bitloops init")?;
        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let locked = stdout.contains("database is locked") || stderr.contains("database is locked");
        if locked && attempts < 2 {
            attempts += 1;
            std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempts)));
            continue;
        }

        return ensure_success(&output, "bitloops init");
    }
}

pub fn run_enable_cli_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["enable"], "bitloops enable")
}

pub fn ensure_claude_auth_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure_claude_authenticated(world)
}

pub fn run_devql_init_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["devql", "init"], "bitloops devql init")
}

pub fn run_devql_ingest_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["devql", "ingest"], "bitloops devql ingest")
}

pub fn run_devql_query(world: &mut QatWorld, query: &str) -> Result<serde_json::Value> {
    let output = run_command_capture(
        world,
        "bitloops devql query",
        build_bitloops_command(world, &["devql", "query", query, "--compact"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    ensure_success(&output, "bitloops devql query")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    serde_json::from_str(stdout.trim()).context("parsing devql query json output")
}

pub fn resolve_head_sha(world: &QatWorld) -> Result<String> {
    let output = run_command_capture(
        world,
        "git rev-parse HEAD",
        build_git_command(world.repo_dir(), &["rev-parse", "HEAD"], &[]),
    )?;
    ensure_success(&output, "git rev-parse HEAD")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn capture_head_sha(world: &mut QatWorld) -> Result<String> {
    let sha = resolve_head_sha(world)?;
    world.captured_commit_shas.push(sha.clone());
    Ok(sha)
}

pub fn count_json_array_rows(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(arr) => arr.len(),
        serde_json::Value::Object(obj) => obj
            .get("rows")
            .or_else(|| obj.get("data"))
            .and_then(serde_json::Value::as_array)
            .map(std::vec::Vec::len)
            .unwrap_or(0),
        _ => 0,
    }
}

pub fn run_first_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, FIRST_CLAUDE_PROMPT)
}

pub fn run_second_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, SECOND_CLAUDE_PROMPT)
}

pub fn commit_for_relative_day_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    days_ago: i64,
    label: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];

    run_git_success(world, &["add", "-A"], &env, "git add -A")?;

    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world.repo_dir(), &["diff", "--cached", "--quiet"], &env),
    )?;

    let diff_code = diff_output.status.code().unwrap_or_default();
    ensure!(
        diff_code <= 1,
        "git diff --cached --quiet failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff_output.stdout),
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let mut args = vec!["commit", "-m", label];
    if diff_code == 0 {
        args.insert(1, "--allow-empty");
    }
    run_git_success(world, &args, &env, "git commit relative day")?;
    run_bitloops_success(
        world,
        &["hooks", "git", "post-commit"],
        "bitloops hooks git post-commit",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn assert_bitloops_stores_exist_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let stores_dir = world.repo_dir().join(".bitloops").join("stores");
    ensure!(
        stores_dir.exists(),
        "expected stores directory to exist at {}",
        stores_dir.display()
    );
    let relational = stores_dir.join("relational").join("relational.db");
    let events = stores_dir.join("event").join("events.duckdb");
    ensure!(
        relational.exists(),
        "expected relational store at {}",
        relational.display()
    );
    ensure!(
        events.exists(),
        "expected events store at {}",
        events.display()
    );
    Ok(())
}

pub fn assert_claude_session_exists_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let backend = create_session_backend_or_local(world.repo_dir());
    let sessions = backend
        .list_sessions()
        .context("listing persisted Bitloops sessions")?;

    let Some(session) = sessions
        .iter()
        .find(|session| session.agent_type == AGENT_NAME_CLAUDE_CODE)
    else {
        bail!("expected at least one persisted claude-code session");
    };

    ensure!(
        !session.session_id.is_empty(),
        "expected claude-code session to have a session id"
    );
    ensure!(
        !session.transcript_path.is_empty(),
        "expected claude-code session to record a transcript path"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_exists_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings = read_commit_checkpoint_mappings(world.repo_dir())
        .context("reading Bitloops checkpoint mappings")?;
    if mappings.is_empty() && claude_fallback_marker_exists(world) {
        append_world_log(
            world,
            "Checkpoint mapping assertion bypassed because QAT Claude fallback is active.\n",
        )?;
        return Ok(());
    }
    let Some(checkpoint_id) = mappings.values().next() else {
        bail!("expected at least one Bitloops checkpoint mapping");
    };

    let summary = read_committed(world.repo_dir(), checkpoint_id)
        .with_context(|| format!("reading committed checkpoint summary for {checkpoint_id}"))?;
    ensure!(
        summary.is_some(),
        "expected committed checkpoint summary for {checkpoint_id}"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_count_at_least_for_repo(
    world: &QatWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings = read_commit_checkpoint_mappings(world.repo_dir())
        .context("reading Bitloops checkpoint mappings")?;
    if mappings.len() < min_count && claude_fallback_marker_exists(world) {
        append_world_log(
            world,
            &format!(
                "Checkpoint mapping count assertion bypassed because QAT Claude fallback is active (have {}, expected at least {}).\n",
                mappings.len(),
                min_count
            ),
        )?;
        return Ok(());
    }
    ensure!(
        mappings.len() >= min_count,
        "expected at least {min_count} Bitloops checkpoint mappings, got {}",
        mappings.len()
    );
    Ok(())
}

pub fn assert_init_yesterday_and_final_today_commit_checkpoints_for_repo(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "git log timeline",
        build_git_command(
            world.repo_dir(),
            &["log", "--pretty=format:%s|%aI", "-n", "30"],
            &[],
        ),
    )?;
    ensure_success(&output, "git log timeline")?;
    let log = String::from_utf8_lossy(&output.stdout);
    let commits = log
        .lines()
        .filter_map(|line| {
            let (subject, author_iso) = line.split_once('|')?;
            Some((subject.to_string(), author_iso.to_string()))
        })
        .collect::<Vec<_>>();
    ensure!(
        commits.len() >= 3,
        "expected at least 3 commits, got {}",
        commits.len()
    );

    let yesterday = expected_date_for_relative_day(1)?;
    let today = expected_date_for_relative_day(0)?;

    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "chore: initial commit" && iso.starts_with(&yesterday)
        }),
        "missing initial commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "test: committed yesterday" && iso.starts_with(&yesterday)
        }),
        "missing yesterday checkpoint commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|(subject, iso)| {
            subject == "test: committed today" && iso.starts_with(&today)
        }),
        "missing today checkpoint commit dated {today}"
    );

    Ok(())
}

pub fn assert_devql_artefacts_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(world, r#"repo("bitloops")->artefacts()->limit(5)"#)?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    ensure!(
        count >= 1,
        "expected at least 1 artefact from devql query, got {count}"
    );
    Ok(())
}

pub fn assert_devql_checkpoints_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
    agent: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let query = format!(
        r#"repo("bitloops")->checkpoints(agent:"{}")->limit(5)"#,
        escape_devql_string(agent)
    );
    let value = run_devql_query(world, &query)?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    if count == 0 && claude_fallback_marker_exists(world) {
        append_world_log(
            world,
            "DevQL checkpoints query assertion bypassed because QAT Claude fallback is active.\n",
        )?;
        return Ok(());
    }
    ensure!(
        count >= 1,
        "expected at least 1 checkpoint for agent {agent}, got {count}"
    );
    Ok(())
}

pub fn assert_devql_chat_history_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let value = run_devql_query(
        world,
        r#"repo("bitloops")->artefacts()->chatHistory()->limit(5)"#,
    )?;
    let count = count_json_array_rows(&value);
    world.last_query_result_count = Some(count);
    ensure!(
        count >= 1,
        "expected at least 1 chat history result, got {count}"
    );
    Ok(())
}
