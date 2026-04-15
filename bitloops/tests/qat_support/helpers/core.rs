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

const QAT_EVENTUAL_TIMEOUT_ENV: &str = "BITLOOPS_QAT_EVENTUAL_TIMEOUT_SECS";
const DEFAULT_QAT_EVENTUAL_TIMEOUT_SECS: u64 = 15;
const QAT_EVENTUAL_POLL_INTERVAL_MILLIS: u64 = 250;

fn qat_eventual_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(QAT_EVENTUAL_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_QAT_EVENTUAL_TIMEOUT_SECS,
    )
}

fn qat_eventual_poll_interval() -> StdDuration {
    StdDuration::from_millis(QAT_EVENTUAL_POLL_INTERVAL_MILLIS)
}

fn wait_for_qat_condition<T, Observe, Ready, Describe>(
    timeout: StdDuration,
    poll_interval: StdDuration,
    expected: &str,
    mut observe: Observe,
    is_ready: Ready,
    describe: Describe,
) -> Result<T>
where
    Observe: FnMut() -> Result<T>,
    Ready: Fn(&T) -> bool,
    Describe: Fn(&T) -> String,
{
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_observation: String;

    loop {
        attempts += 1;
        match observe() {
            Ok(value) => {
                let summary = describe(&value);
                if is_ready(&value) {
                    return Ok(value);
                }
                last_observation = format!("value: {summary}");
            }
            Err(err) => {
                last_observation = format!("error: {err:#}");
            }
        }

        if started.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for {expected}; attempts={attempts}; last observation={}",
                timeout.as_secs(),
                last_observation
            );
        }

        std::thread::sleep(poll_interval);
    }
}

pub fn ensure_daemon_for_scenario(world: &mut QatWorld) -> Result<()> {
    stop_daemon_for_scenario(world).ok();

    let stderr_log_path = daemon_stderr_log_path(world.run_dir());
    let mut attempt_errors = Vec::new();
    for port in daemon_candidate_ports(world.run_dir()) {
        append_world_log(
            world,
            &format!("Starting foreground daemon for scenario using port candidate {port}.\n"),
        )?;

        let mut child = spawn_daemon_process(world, &port, &stderr_log_path)?;
        match wait_for_daemon_ready(world.run_dir(), &mut child, &stderr_log_path) {
            Ok((runtime_state_path, runtime_state)) => {
                append_world_log(
                    world,
                    &format!(
                        "Daemon ready for scenario on {} (pid {}, requested port {}).\nRuntime state: {}\nStderr log: {}\n",
                        runtime_state.url,
                        runtime_state.pid,
                        port,
                        runtime_state_path.display(),
                        stderr_log_path.display()
                    ),
                )?;
                world.daemon_url = Some(runtime_state.url.clone());
                world.daemon_runtime_state_path = Some(runtime_state_path.clone());
                world.daemon_stderr_log_path = Some(stderr_log_path.clone());
                world.daemon_process = Some(crate::qat_support::world::ScenarioDaemonProcess {
                    child,
                    requested_port: port,
                    stderr_log_path,
                    runtime_state_path,
                });
                return Ok(());
            }
            Err(err) => {
                append_world_log(
                    world,
                    &format!("Daemon startup attempt failed for port candidate {port}: {err:#}\n"),
                )?;
                let _ = child.kill();
                let _ = child.wait();
                attempt_errors.push(format!("port {port}: {err:#}"));
            }
        }
    }

    bail!(
        "failed to bootstrap and start daemon for QAT scenario\n{}",
        attempt_errors.join("\n\n")
    );
}

fn error_chain_contains_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
    })
}

pub fn stop_daemon_for_scenario(world: &mut QatWorld) -> Result<()> {
    if world.run_dir.is_none() || world.repo_dir.is_none() || world.terminal_log_path.is_none() {
        return Ok(());
    }

    let had_daemon = world.daemon_process.is_some() || world.daemon_url.is_some();
    let mut stop_error = None;

    if had_daemon {
        match run_command_capture(
            world,
            "bitloops daemon stop",
            build_bitloops_command(world, &["daemon", "stop"])?,
        ) {
            Ok(output) if output.status.success() => {
                append_world_log(world, "Daemon stopped for scenario via CLI.\n")?;
            }
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                append_world_log(
                    world,
                    &format!(
                        "Daemon stop returned non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}\n",
                    ),
                )?;
                stop_error = Some(anyhow!(
                    "bitloops daemon stop returned non-zero\nstdout:\n{stdout}\nstderr:\n{stderr}"
                ));
            }
            Err(err) if error_chain_contains_not_found(&err) => {
                append_world_log(
                    world,
                    "Daemon stop skipped because the bitloops binary is no longer present.\n",
                )?;
            }
            Err(err) => {
                append_world_log(world, &format!("Daemon stop failed: {err:#}\n"))?;
                stop_error = Some(err.context("running bitloops daemon stop"));
            }
        }
    }

    if let Some(mut process) = world.daemon_process.take() {
        match process.child.try_wait() {
            Ok(Some(status)) => {
                append_world_log(
                    world,
                    &format!("Foreground daemon child already exited with status {status}.\n"),
                )?;
            }
            Ok(None) => {
                append_world_log(
                    world,
                    &format!(
                        "Foreground daemon child still running after stop; terminating pid {}.\n",
                        process.child.id()
                    ),
                )?;
                if let Err(err) = process.child.kill() {
                    append_world_log(
                        world,
                        &format!("Failed to terminate foreground daemon child: {err}\n"),
                    )?;
                }
                if let Err(err) = process.child.wait() {
                    append_world_log(
                        world,
                        &format!("Failed waiting for foreground daemon child: {err}\n"),
                    )?;
                }
            }
            Err(err) => {
                append_world_log(
                    world,
                    &format!("Failed to inspect foreground daemon child: {err}\n"),
                )?;
            }
        }
    }

    world.daemon_url = None;
    world.daemon_runtime_state_path = None;
    world.daemon_stderr_log_path = None;

    if let Some(err) = stop_error {
        return Err(err);
    }

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
        build_git_command(world, &["init", "-q"], &[]),
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
    run_init_bitloops_with_agent(world, repo_name, "claude-code", false, None)
}

fn normalise_onboarding_agent_name(agent_name: &str) -> &str {
    if agent_name.eq_ignore_ascii_case("claude") {
        AGENT_NAME_CLAUDE_CODE
    } else if agent_name.eq_ignore_ascii_case("open-code") {
        AGENT_NAME_OPEN_CODE
    } else {
        agent_name
    }
}

fn normalise_smoke_agent_name(agent_name: &str) -> &str {
    let normalised = normalise_onboarding_agent_name(agent_name);
    if normalised.eq_ignore_ascii_case(AGENT_NAME_CLAUDE_CODE) {
        AGENT_NAME_CLAUDE_CODE
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_CURSOR) {
        AGENT_NAME_CURSOR
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_GEMINI) {
        AGENT_NAME_GEMINI
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_COPILOT) {
        AGENT_NAME_COPILOT
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_CODEX) {
        AGENT_NAME_CODEX
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_OPEN_CODE) {
        AGENT_NAME_OPEN_CODE
    } else {
        normalised
    }
}

pub fn run_init_bitloops_with_agent(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    force: bool,
    sync: Option<bool>,
) -> Result<()> {
    run_init_bitloops_with_agent_config(world, repo_name, agent_name, force, sync, None, None)
}

pub fn run_init_bitloops_with_agent_sync_ingest_backfill(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    sync: bool,
    ingest: bool,
    backfill: usize,
) -> Result<()> {
    run_init_bitloops_with_agent_config(
        world,
        repo_name,
        agent_name,
        false,
        Some(sync),
        Some(ingest),
        Some(backfill),
    )
}

fn run_init_bitloops_with_agent_config(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    force: bool,
    sync: Option<bool>,
    ingest: Option<bool>,
    backfill: Option<usize>,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_onboarding_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());

    let args_owned =
        build_init_bitloops_args_with_options(normalised_agent_name, force, sync, ingest, backfill);
    let label = format!("bitloops {}", args_owned.join(" "));
    let mut attempts = 0_u8;

    loop {
        let args: Vec<&str> = args_owned.iter().map(String::as_str).collect();
        let output = run_command_capture(world, &label, build_bitloops_command(world, &args)?)
            .with_context(|| format!("running {label}"))?;
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

        return ensure_success(&output, &label);
    }
}

fn build_init_bitloops_args(agent_name: &str, force: bool, sync: Option<bool>) -> Vec<String> {
    build_init_bitloops_args_with_options(agent_name, force, sync, None, None)
}

fn build_init_bitloops_args_with_options(
    agent_name: &str,
    force: bool,
    sync: Option<bool>,
    ingest: Option<bool>,
    backfill: Option<usize>,
) -> Vec<String> {
    let mut args = vec![
        "init".to_string(),
        "--agent".to_string(),
        agent_name.to_string(),
    ];

    let sync_choice = sync.unwrap_or(false);
    let ingest_choice = if backfill.is_some() {
        true
    } else {
        ingest.unwrap_or(false)
    };
    args.push(format!("--sync={sync_choice}"));
    args.push(format!("--ingest={ingest_choice}"));
    if let Some(backfill_window) = backfill {
        args.push(format!("--backfill={backfill_window}"));
    }

    if force {
        args.push("--force".to_string());
    }

    args
}

pub fn run_enable_cli_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_enable(world, &["enable"], "bitloops enable")
}

pub fn run_bitloops_enable_with_flags(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut args = vec!["enable"];
    args.extend_from_slice(flags);
    let label = format!("bitloops {}", args.join(" "));
    run_enable(world, &args, &label)
}

fn run_enable(world: &mut QatWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    ensure_success(&output, label)
}

pub fn run_bitloops_disable(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["disable"], "bitloops disable")
}

pub fn run_bitloops_uninstall_full(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(
        world,
        &["uninstall", "--full", "-f"],
        "bitloops uninstall --full",
    )
}

pub fn assert_bitloops_binary_removed(world: &mut QatWorld) -> Result<()> {
    let mut cmd = build_bitloops_command(world, &["--version"])?;
    match cmd.output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Binary not found — this is the expected outcome after full uninstall
            Ok(())
        }
        Err(e) => bail!("unexpected error running bitloops --version: {e}"),
        Ok(output) => {
            ensure!(
                !output.status.success(),
                "expected bitloops --version to fail after full uninstall, but it exited with code 0"
            );
            Ok(())
        }
    }
}

pub fn run_bitloops_uninstall_hooks(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(
        world,
        &[
            "uninstall",
            "--agent-hooks",
            "--git-hooks",
            "--only-current-project",
            "-f",
        ],
        "bitloops uninstall --agent-hooks --git-hooks",
    )
}

pub fn ensure_claude_auth_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure_claude_authenticated(world)
}

pub fn run_devql_init_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["devql", "init"], "bitloops devql init")
}

pub fn run_devql_ingest_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql tasks enqueue --kind ingest --status",
        build_bitloops_command(
            world,
            &["devql", "tasks", "enqueue", "--kind", "ingest", "--status"],
        )?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout);
    ensure_success(
        &output,
        "bitloops devql tasks enqueue --kind ingest --status",
    )
}

pub fn assert_version_output(world: &mut QatWorld) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops --version",
        build_bitloops_command(world, &["--version"])?,
    )?;
    ensure_success(&output, "bitloops --version")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let re =
        regex::Regex::new(r"Bitloops CLI v\d+\.\d+\.\d+").context("compiling version regex")?;
    ensure!(
        re.is_match(&stdout),
        "expected semver in version output, got:\n{}",
        stdout
    );
    Ok(())
}

pub fn assert_daemon_config_exists(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    ensure!(
        macos_config.exists() || xdg_config.exists(),
        "expected daemon config at {} or {}",
        macos_config.display(),
        xdg_config.display()
    );
    Ok(())
}

pub fn assert_config_has_relational_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.relational]"),
        "daemon config missing [stores.relational] section:\n{}",
        content
    );
    ensure!(
        content.contains("sqlite_path") || content.contains("postgres_dsn"),
        "daemon config missing relational store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_config_has_event_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.events]"),
        "daemon config missing [stores.events] section:\n{}",
        content
    );
    ensure!(
        content.contains("duckdb_path"),
        "daemon config missing event store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_config_has_blob_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.blob]"),
        "daemon config missing [stores.blob] section:\n{}",
        content
    );
    ensure!(
        content.contains("local_path"),
        "daemon config missing blob store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_store_paths_exist(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let resolved = resolve_daemon_config(Some(&config_path))
        .with_context(|| format!("resolving daemon config from {}", config_path.display()))?;

    ensure!(
        resolved.relational_db_path.is_file(),
        "SQLite store file does not exist at {}",
        resolved.relational_db_path.display()
    );
    ensure!(
        resolved.events_db_path.is_file(),
        "DuckDB store file does not exist at {}",
        resolved.events_db_path.display()
    );
    ensure!(
        resolved.blob_store_path.is_dir(),
        "Blob store directory does not exist at {}",
        resolved.blob_store_path.display()
    );
    Ok(())
}

pub fn assert_file_exists_in_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let full_path = world.repo_dir().join(relative_path);
    ensure!(
        full_path.exists(),
        "expected path to exist: {}",
        full_path.display()
    );
    Ok(())
}

pub fn assert_agent_hooks_installed(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    match normalised_agent_name {
        AGENT_NAME_CODEX => {
            let path = world.repo_dir().join(".codex").join("hooks.json");
            ensure!(path.exists(), "expected {}", path.display());
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            ensure!(
                content.contains("bitloops hooks codex session-start"),
                "missing codex session-start hook in {}",
                path.display()
            );
            ensure!(
                content.contains("bitloops hooks codex stop"),
                "missing codex stop hook in {}",
                path.display()
            );
        }
        AGENT_NAME_CLAUDE_CODE => {
            let path = world.repo_dir().join(".claude").join("settings.json");
            ensure!(path.exists(), "expected {}", path.display());
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            ensure!(
                content.contains("bitloops hooks claude-code stop"),
                "missing claude-code stop hook in {}",
                path.display()
            );
        }
        AGENT_NAME_CURSOR => {
            let path = world.repo_dir().join(".cursor").join("hooks.json");
            ensure!(path.exists(), "expected {}", path.display());
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            ensure!(
                content.contains("bitloops"),
                "missing bitloops hook in {}",
                path.display()
            );
        }
        AGENT_NAME_GEMINI => {
            let path = world.repo_dir().join(".gemini").join("settings.json");
            ensure!(path.exists(), "expected {}", path.display());
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            ensure!(
                content.contains("bitloops"),
                "missing bitloops hook in {}",
                path.display()
            );
        }
        AGENT_NAME_COPILOT => {
            let path = world
                .repo_dir()
                .join(".github")
                .join("hooks")
                .join("bitloops.json");
            ensure!(path.exists(), "expected {}", path.display());
        }
        AGENT_NAME_OPEN_CODE => {
            let path = world
                .repo_dir()
                .join(".opencode")
                .join("plugins")
                .join("bitloops.ts");
            ensure!(path.exists(), "expected {}", path.display());
        }
        other => bail!("unsupported agent for hook assertion: {other}"),
    }

    let post_commit_path = world
        .repo_dir()
        .join(".git")
        .join("hooks")
        .join("post-commit");
    ensure!(
        post_commit_path.exists(),
        "expected git post-commit hook at {}",
        post_commit_path.display()
    );
    let post_commit_content = fs::read_to_string(&post_commit_path)
        .with_context(|| format!("reading {}", post_commit_path.display()))?;
    ensure!(
        post_commit_content.contains("bitloops hooks git post-commit"),
        "missing post-commit bitloops hook in {}",
        post_commit_path.display()
    );
    Ok(())
}

pub fn assert_agent_hooks_removed(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let hooks_path = match normalised_agent_name {
        AGENT_NAME_CLAUDE_CODE => world.repo_dir().join(".claude").join("settings.json"),
        AGENT_NAME_CODEX => world.repo_dir().join(".codex").join("hooks.json"),
        AGENT_NAME_CURSOR => world.repo_dir().join(".cursor").join("hooks.json"),
        AGENT_NAME_GEMINI => world.repo_dir().join(".gemini").join("settings.json"),
        AGENT_NAME_COPILOT => world
            .repo_dir()
            .join(".github")
            .join("hooks")
            .join("bitloops.json"),
        AGENT_NAME_OPEN_CODE => world
            .repo_dir()
            .join(".opencode")
            .join("plugins")
            .join("bitloops.ts"),
        other => bail!("unsupported agent for hook removal assertion: {other}"),
    };
    if hooks_path.exists() {
        let content = fs::read_to_string(&hooks_path)
            .with_context(|| format!("reading {}", hooks_path.display()))?;
        ensure!(
            !content.contains("bitloops"),
            "agent hooks file still contains bitloops references after uninstall: {}",
            hooks_path.display()
        );
    }
    Ok(())
}

pub fn assert_git_hooks_removed(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let post_commit_path = world
        .repo_dir()
        .join(".git")
        .join("hooks")
        .join("post-commit");
    if post_commit_path.exists() {
        let content = fs::read_to_string(&post_commit_path)
            .with_context(|| format!("reading {}", post_commit_path.display()))?;
        ensure!(
            !content.contains("bitloops hooks git post-commit"),
            "git post-commit hook still contains bitloops after uninstall: {}",
            post_commit_path.display()
        );
    }
    Ok(())
}

pub fn assert_status_shows_disabled(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops checkpoints status",
        build_bitloops_command(world, &["checkpoints", "status"])?,
    )?;
    ensure_success(&output, "bitloops checkpoints status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        stdout.contains("Disabled"),
        "expected disabled status output, got:\n{}",
        stdout
    );
    let settings =
        load_settings(world.repo_dir()).context("loading repo settings after bitloops disable")?;
    ensure!(
        !settings.enabled,
        "expected capture.enabled=false after disable, but settings report enabled=true"
    );
    Ok(())
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

pub fn run_devql_graphql_query(world: &mut QatWorld, query: &str) -> Result<serde_json::Value> {
    let output = run_command_capture(
        world,
        "bitloops devql query --graphql",
        build_bitloops_command(world, &["devql", "query", "--graphql", query, "--compact"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    ensure_success(&output, "bitloops devql query --graphql")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    serde_json::from_str(stdout.trim()).context("parsing raw DevQL GraphQL json output")
}

pub fn resolve_head_sha(world: &QatWorld) -> Result<String> {
    let output = run_command_capture(
        world,
        "git rev-parse HEAD",
        build_git_command(world, &["rev-parse", "HEAD"], &[]),
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

fn count_artefacts_across_source_files(world: &mut QatWorld) -> Result<usize> {
    if let Ok(value) = run_devql_query(world, r#"repo("bitloops")->artefacts()->limit(500)"#) {
        let count = count_json_array_rows(&value);
        if count > 0 {
            return Ok(count);
        }
    }

    let mut pending = vec![world.repo_dir().to_path_buf()];
    let mut file_paths = Vec::new();
    while let Some(dir) = pending.pop() {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("reading source directory {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if matches!(
                    dir_name,
                    ".git" | ".bitloops" | "node_modules" | "target" | "dist"
                ) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !matches!(extension, "ts" | "tsx" | "js" | "jsx" | "rs" | "py") {
                continue;
            }
            let relative = path
                .strip_prefix(world.repo_dir())
                .with_context(|| format!("making path relative for {}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            file_paths.push(relative);
        }
    }

    let mut total = 0;
    for file_path in file_paths {
        let query = format!(
            r#"repo("bitloops")->file("{}")->artefacts()->limit(500)"#,
            escape_devql_string(&file_path)
        );
        let value = match run_devql_query(world, &query) {
            Ok(value) => value,
            Err(err) => {
                let message = err.to_string();
                if message.contains("missing string field `canonical_kind`")
                    || message.contains("unknown path")
                {
                    append_world_log(
                        world,
                        &format!(
                            "Skipping artefacts count for `{file_path}` due unsupported path or backend mismatch.\n"
                        ),
                    )?;
                    continue;
                }
                return Err(err);
            }
        };
        total += count_json_array_rows(&value);
    }
    Ok(total)
}

pub fn run_first_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_first_change_using_agent_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

pub fn run_claude_code_prompt_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    prompt: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, prompt)
}

pub fn run_second_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_second_change_using_agent_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

pub fn run_first_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    run_change_using_agent_for_repo(world, repo_name, agent_name, FIRST_CLAUDE_PROMPT)
}

pub fn run_second_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    run_change_using_agent_for_repo(world, repo_name, agent_name, SECOND_CLAUDE_PROMPT)
}

fn run_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    prompt: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());

    match normalised_agent_name {
        AGENT_NAME_CLAUDE_CODE => run_deterministic_claude_smoke_prompt(world, prompt),
        AGENT_NAME_CURSOR => run_cursor_prompt(world, prompt),
        AGENT_NAME_GEMINI => run_gemini_prompt(world, prompt),
        AGENT_NAME_COPILOT => run_copilot_prompt(world, prompt),
        AGENT_NAME_CODEX => run_codex_prompt(world, prompt),
        AGENT_NAME_OPEN_CODE => run_opencode_prompt(world, prompt),
        other => bail!("unsupported smoke agent: {other}"),
    }
}

// ── DevQL sync helpers ───────────────────────────────────────

const DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_ENV: &str =
    "BITLOOPS_QAT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS";
const DEFAULT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS: u64 = 60;
const DAEMON_CAPABILITY_EVENT_STATUS_POLL_INTERVAL_MILLIS: u64 = 250;

pub fn run_devql_sync_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_devql_sync_command(world, repo_name, &[], true)
}

pub fn run_devql_sync_without_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_devql_sync_command(world, repo_name, &[], false)
}

pub fn run_devql_sync_with_flags(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
) -> Result<()> {
    run_devql_sync_command(world, repo_name, flags, true)
}

fn run_devql_sync_command(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
    include_status: bool,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut args = vec!["devql", "tasks", "enqueue", "--kind", "sync"];
    args.extend_from_slice(flags);
    if include_status && !args.contains(&"--status") {
        args.push("--status");
    }
    let label = format!("bitloops {}", args.join(" "));
    let output = run_command_capture(world, &label, build_bitloops_command(world, &args)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout);
    ensure_success(&output, &label)
}

pub fn attempt_devql_sync(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql tasks enqueue --kind sync (expect failure)",
        build_bitloops_command(world, &["devql", "tasks", "enqueue", "--kind", "sync"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(format!("{stdout}\n{stderr}"));
    Ok(())
}

pub fn add_new_rust_source_file(world: &QatWorld) -> Result<()> {
    let file_path = world.repo_dir().join("src").join("lib.rs");
    fs::write(
        &file_path,
        "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn modify_rust_main(world: &QatWorld) -> Result<()> {
    let file_path = world.repo_dir().join("src").join("main.rs");
    fs::write(
        &file_path,
        "mod lib;\n\nfn main() {\n    println!(\"{}\", lib::greet(\"Bitloops\"));\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn delete_rust_source_file(world: &QatWorld) -> Result<()> {
    let candidates = [
        world.repo_dir().join("src").join("lib.rs"),
        world.repo_dir().join("src").join("main.rs"),
    ];
    for path in &candidates {
        if path.exists() {
            fs::remove_file(path).with_context(|| format!("deleting {}", path.display()))?;
            return Ok(());
        }
    }
    bail!("no known Rust source file found to delete")
}

pub fn commit_without_hooks(world: &mut QatWorld) -> Result<()> {
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world, &["diff", "--cached", "--quiet"], &[]),
    )?;
    let diff_code = diff_output.status.code().unwrap_or_default();
    let mut args = vec!["commit", "-m", "QAT change (no hooks)"];
    if diff_code == 0 {
        args.insert(1, "--allow-empty");
    }
    run_git_success(world, &args, &[], "git commit (no hooks)")?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn stage_changes_without_committing(world: &QatWorld) -> Result<()> {
    let output = run_command_capture(
        world,
        "git add -A (stage only)",
        build_git_command(world, &["add", "-A"], &[]),
    )?;
    ensure_success(&output, "git add -A (stage only)")
}

pub fn simulate_git_pull_with_changes(world: &mut QatWorld) -> Result<()> {
    run_git_success(
        world,
        &["checkout", "-b", "qat-remote-changes"],
        &[],
        "git checkout -b qat-remote-changes",
    )?;
    let file_path = world.repo_dir().join("src").join("utils.rs");
    fs::write(
        &file_path,
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", "feat: add utils module from remote"],
        &[],
        "git commit utils",
    )?;
    run_git_success(
        world,
        &["checkout", "-"],
        &[],
        "git checkout previous branch",
    )?;
    run_git_success(
        world,
        &[
            "merge",
            "qat-remote-changes",
            "--no-ff",
            "-m",
            "Merge remote changes",
        ],
        &[],
        "git merge remote changes",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn create_branch_with_additional_files(world: &mut QatWorld) -> Result<()> {
    run_git_success(
        world,
        &["checkout", "-b", "qat-feature-branch"],
        &[],
        "git checkout -b qat-feature-branch",
    )?;
    let file_path = world.repo_dir().join("src").join("config.rs");
    fs::write(
        &file_path,
        "pub const APP_NAME: &str = \"qat-sample\";\npub const VERSION: &str = \"0.1.0\";\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn run_devql_sync_validate_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_devql_sync_with_flags(world, repo_name, &["--validate"])
}

pub fn wait_for_test_harness_capability_event_completion_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let timeout = parse_timeout_seconds(
        std::env::var(DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_ENV)
            .ok()
            .as_deref(),
        DEFAULT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS,
    );
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_payload = serde_json::json!({});

    loop {
        attempts += 1;
        let output = run_command_capture(
            world,
            "bitloops daemon status --json",
            build_bitloops_command(world, &["daemon", "status", "--json"])?,
        )?;
        world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        world.last_command_stdout = Some(stdout.clone());
        ensure_success(&output, "bitloops daemon status --json")?;

        let payload: serde_json::Value = serde_json::from_str(stdout.trim())
            .context("parsing bitloops daemon status --json output")?;
        if let Some(current_repo_run) = payload
            .get("capability_events")
            .and_then(|value| value.get("current_repo_run"))
            .filter(|current_repo_run| {
                current_repo_run
                    .get("capability_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("test_harness")
            })
        {
            match current_repo_run
                .get("status")
                .and_then(serde_json::Value::as_str)
            {
                Some("completed") => return Ok(()),
                Some("failed") => {
                    let run_id = current_repo_run
                        .get("run_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<unknown>");
                    let handler_id = current_repo_run
                        .get("handler_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<unknown>");
                    let event_kind = current_repo_run
                        .get("event_kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<unknown>");
                    let error = current_repo_run
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<no error>");
                    bail!(
                        "test_harness capability event run failed while waiting for completion: run_id={run_id}; handler_id={handler_id}; event_kind={event_kind}; error={error}"
                    );
                }
                _ => {}
            }
        }

        if let Some((_, persisted_run)) = load_latest_test_harness_capability_event_run(world)? {
            match persisted_run.status {
                bitloops::daemon::CapabilityEventRunStatus::Completed => return Ok(()),
                bitloops::daemon::CapabilityEventRunStatus::Failed => {
                    let run_id = persisted_run.run_id;
                    let handler_id = persisted_run.handler_id;
                    let event_kind = persisted_run.event_kind;
                    let error = persisted_run
                        .error
                        .unwrap_or_else(|| "<no error>".to_string());
                    bail!(
                        "test_harness capability event run failed while waiting for completion: run_id={run_id}; handler_id={handler_id}; event_kind={event_kind}; error={error}"
                    );
                }
                _ => {}
            }
        }

        last_payload = payload;
        if started.elapsed() >= timeout {
            let last_payload = serde_json::to_string(&last_payload)
                .unwrap_or_else(|_| "<failed to serialize payload>".to_string());
            bail!(
                "timed out after {}s waiting for test_harness capability-event completion in {}; attempts={attempts}; last payload={last_payload}",
                timeout.as_secs(),
                repo_name
            );
        }
        std::thread::sleep(StdDuration::from_millis(
            DAEMON_CAPABILITY_EVENT_STATUS_POLL_INTERVAL_MILLIS,
        ));
    }
}

fn load_latest_test_harness_capability_event_run(
    world: &QatWorld,
) -> Result<
    Option<(
        std::path::PathBuf,
        bitloops::daemon::CapabilityEventRunRecord,
    )>,
> {
    let candidates = daemon_runtime_store_candidate_paths(world.run_dir());

    let mut latest: Option<(
        std::path::PathBuf,
        bitloops::daemon::CapabilityEventRunRecord,
    )> = None;
    for path in &candidates {
        if !path.exists() {
            continue;
        }

        let store = bitloops::host::runtime_store::DaemonSqliteRuntimeStore::open_at(path.clone())
            .with_context(|| format!("opening daemon runtime store {}", path.display()))?;
        let Some(run) = latest_capability_event_run(
            load_latest_test_harness_current_state_run(&store)?,
            load_latest_test_harness_pack_reconcile_run(&store)?,
        ) else {
            continue;
        };

        let replace = latest.as_ref().is_none_or(|(_, current)| {
            capability_event_run_sort_key(&run) > capability_event_run_sort_key(current)
        });
        if replace {
            latest = Some((path.clone(), run));
        }
    }

    Ok(latest)
}

fn latest_capability_event_run(
    current_state_run: Option<bitloops::daemon::CapabilityEventRunRecord>,
    legacy_run: Option<bitloops::daemon::CapabilityEventRunRecord>,
) -> Option<bitloops::daemon::CapabilityEventRunRecord> {
    match (current_state_run, legacy_run) {
        (Some(current_state), Some(legacy)) => {
            if capability_event_run_sort_key(&legacy)
                > capability_event_run_sort_key(&current_state)
            {
                Some(legacy)
            } else {
                Some(current_state)
            }
        }
        (Some(current_state), None) => Some(current_state),
        (None, Some(legacy)) => Some(legacy),
        (None, None) => None,
    }
}

fn capability_event_run_sort_key(
    run: &bitloops::daemon::CapabilityEventRunRecord,
) -> (u64, u64, u64) {
    (
        run.updated_at_unix,
        run.completed_at_unix.unwrap_or_default(),
        run.submitted_at_unix,
    )
}

fn load_latest_test_harness_current_state_run(
    store: &bitloops::host::runtime_store::DaemonSqliteRuntimeStore,
) -> Result<Option<bitloops::daemon::CapabilityEventRunRecord>> {
    use rusqlite::OptionalExtension;

    store.with_connection(|conn| {
        conn.query_row(
            "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error \
             FROM capability_workplane_cursor_runs \
             WHERE capability_id = ?1 AND mailbox_name = ?2 \
             ORDER BY updated_at_unix DESC, submitted_at_unix DESC \
             LIMIT 1",
            rusqlite::params!["test_harness", "test_harness.current_state"],
            |row| {
                let as_u64 = |index| -> rusqlite::Result<u64> {
                    row.get::<_, i64>(index)
                        .map(|value| u64::try_from(value).unwrap_or_default())
                };
                let opt_u64 = |index| -> rusqlite::Result<Option<u64>> {
                    row.get::<_, Option<i64>>(index).map(|value| {
                        value.and_then(|value| u64::try_from(value).ok())
                    })
                };
                let repo_id = row.get::<_, String>(1)?;
                let consumer_id = row.get::<_, String>(3)?;

                Ok(bitloops::daemon::CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: repo_id.clone(),
                    capability_id: row.get(4)?,
                    consumer_id: consumer_id.clone(),
                    handler_id: consumer_id.clone(),
                    from_generation_seq: as_u64(5)?,
                    to_generation_seq: as_u64(6)?,
                    reconcile_mode: row.get(7)?,
                    event_kind: "current_state_consumer".to_string(),
                    lane_key: format!("{repo_id}:{consumer_id}"),
                    event_payload_json: String::new(),
                    status: parse_capability_event_run_status(&row.get::<_, String>(8)?)?,
                    attempts: row.get(9)?,
                    submitted_at_unix: as_u64(10)?,
                    started_at_unix: opt_u64(11)?,
                    updated_at_unix: as_u64(12)?,
                    completed_at_unix: opt_u64(13)?,
                    error: row.get(14)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn load_latest_test_harness_pack_reconcile_run(
    store: &bitloops::host::runtime_store::DaemonSqliteRuntimeStore,
) -> Result<Option<bitloops::daemon::CapabilityEventRunRecord>> {
    use rusqlite::OptionalExtension;

    store.with_connection(|conn| {
        conn.query_row(
            "SELECT run_id, repo_id, capability_id, consumer_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error \
             FROM pack_reconcile_runs \
             WHERE capability_id = ?1 AND consumer_id = ?2 \
             ORDER BY updated_at_unix DESC, submitted_at_unix DESC \
             LIMIT 1",
            rusqlite::params!["test_harness", "test_harness.current_state"],
            |row| {
                let as_u64 = |index| -> rusqlite::Result<u64> {
                    row.get::<_, i64>(index)
                        .map(|value| u64::try_from(value).unwrap_or_default())
                };
                let opt_u64 = |index| -> rusqlite::Result<Option<u64>> {
                    row.get::<_, Option<i64>>(index).map(|value| {
                        value.and_then(|value| u64::try_from(value).ok())
                    })
                };
                let repo_id = row.get::<_, String>(1)?;
                let consumer_id = row.get::<_, String>(3)?;

                Ok(bitloops::daemon::CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: repo_id.clone(),
                    capability_id: row.get(2)?,
                    consumer_id: consumer_id.clone(),
                    handler_id: consumer_id.clone(),
                    from_generation_seq: as_u64(4)?,
                    to_generation_seq: as_u64(5)?,
                    reconcile_mode: row.get(6)?,
                    event_kind: String::new(),
                    lane_key: format!("{repo_id}:{consumer_id}"),
                    event_payload_json: String::new(),
                    status: parse_capability_event_run_status(&row.get::<_, String>(7)?)?,
                    attempts: row.get(8)?,
                    submitted_at_unix: as_u64(9)?,
                    started_at_unix: opt_u64(10)?,
                    updated_at_unix: as_u64(11)?,
                    completed_at_unix: opt_u64(12)?,
                    error: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn parse_capability_event_run_status(
    value: &str,
) -> rusqlite::Result<bitloops::daemon::CapabilityEventRunStatus> {
    match value {
        "queued" => Ok(bitloops::daemon::CapabilityEventRunStatus::Queued),
        "running" => Ok(bitloops::daemon::CapabilityEventRunStatus::Running),
        "completed" => Ok(bitloops::daemon::CapabilityEventRunStatus::Completed),
        "failed" => Ok(bitloops::daemon::CapabilityEventRunStatus::Failed),
        "cancelled" => Ok(bitloops::daemon::CapabilityEventRunStatus::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!(
                "unknown capability-event run status `{other}`"
            ))),
        )),
    }
}

fn resolve_qat_sync_state_path(
    world: &QatWorld,
) -> Result<(
    std::path::PathBuf,
    bitloops::host::runtime_store::PersistedDevqlTaskQueueState,
)> {
    let candidates = daemon_runtime_store_candidate_paths(world.run_dir());

    for path in &candidates {
        if !path.exists() {
            continue;
        }
        let store = bitloops::host::runtime_store::DaemonSqliteRuntimeStore::open_at(path.clone())
            .with_context(|| format!("opening daemon runtime store {}", path.display()))?;
        if let Some(state) = store
            .load_devql_task_queue_state()
            .with_context(|| format!("loading DevQL task queue state from {}", path.display()))?
        {
            return Ok((path.clone(), state));
        }
    }

    bail!(
        "could not find daemon DevQL task queue state in runtime store; looked in: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

type CompletedHeadTasks = (
    String,
    std::path::PathBuf,
    Vec<(String, bitloops::host::devql::SyncSummary)>,
);

fn completed_tasks_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<CompletedHeadTasks> {
    ensure_bitloops_repo_name(repo_name)?;
    let head_output = run_command_capture(
        world,
        "git rev-parse HEAD (sync history assertion)",
        build_git_command(world, &["rev-parse", "HEAD"], &[]),
    )?;
    ensure_success(&head_output, "git rev-parse HEAD (sync history assertion)")?;

    let head_sha = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();
    ensure!(
        !head_sha.is_empty(),
        "expected non-empty HEAD SHA for sync history assertion"
    );

    let (sync_state_path, snapshot) = resolve_qat_sync_state_path(world)?;

    let head_tasks: Vec<(String, bitloops::host::devql::SyncSummary)> = snapshot
        .tasks
        .into_iter()
        .filter(|task| task.kind == bitloops::daemon::DevqlTaskKind::Sync)
        .filter(|task| task.status == bitloops::daemon::DevqlTaskStatus::Completed)
        .filter_map(|task| {
            let source = task.source.to_string();
            task.sync_result().cloned().map(|summary| (source, summary))
        })
        .filter(|(_, summary)| summary.head_commit_sha.as_deref() == Some(head_sha.as_str()))
        .collect();

    ensure!(
        !head_tasks.is_empty(),
        "expected at least one completed sync task for HEAD `{head_sha}` in {}; found none",
        sync_state_path.display()
    );

    Ok((head_sha, sync_state_path, head_tasks))
}

fn format_task_diagnostics(head_tasks: &[(String, bitloops::host::devql::SyncSummary)]) -> String {
    head_tasks
        .iter()
        .map(|(source, summary)| {
            format!(
                "source={} mode={} added={} changed={} removed={} unchanged={}",
                source,
                summary.mode,
                summary.paths_added,
                summary.paths_changed,
                summary.paths_removed,
                summary.paths_unchanged
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn assert_sync_history_has_added_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_added > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsAdded > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_changed_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_changed > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsChanged > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_removed_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_removed > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsRemoved > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_artefacts_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_added + summary.paths_unchanged > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with artefacts indexed (pathsAdded + pathsUnchanged > 0) for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

/// Parse a numeric field from the sync summary output.
/// Format: "sync complete: 5 added, 0 changed, 0 removed, 3 unchanged, 3 cache hits (1 cache misses, 0 parse errors)"
pub fn parse_sync_summary_field(stdout: &str, field: &str) -> Option<usize> {
    for segment in stdout.split([',', '(', ')']) {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed.strip_suffix(field) {
            let number_str = rest
                .trim()
                .rsplit(' ')
                .next()
                .unwrap_or("")
                .trim_end_matches(':');
            if let Ok(value) = number_str.parse::<usize>() {
                return Some(value);
            }
        }
    }
    None
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
        build_git_command(world, &["diff", "--cached", "--quiet"], &env),
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
    let repo_stores_dir = world.repo_dir().join(".bitloops").join("stores");
    let (stores_dir, relational, events) = if repo_stores_dir.exists() {
        (
            repo_stores_dir.clone(),
            repo_stores_dir.join("relational").join("relational.db"),
            repo_stores_dir.join("event").join("events.duckdb"),
        )
    } else {
        with_scenario_app_env(world, || {
            let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
                .context("resolving store backend config for QAT store assertions")?;
            let relational = resolve_sqlite_db_path_for_repo(
                world.repo_dir(),
                cfg.relational.sqlite_path.as_deref(),
            )
            .context("resolving relational store path for QAT store assertions")?;
            let events = resolve_duckdb_db_path_for_repo(
                world.repo_dir(),
                cfg.events.duckdb_path.as_deref(),
            );
            let stores_dir = relational
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            Ok::<_, anyhow::Error>((stores_dir, relational, events))
        })?
    };
    ensure!(
        stores_dir.exists(),
        "expected stores directory to exist at {}",
        stores_dir.display()
    );
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
    assert_agent_session_exists_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

pub fn assert_agent_session_exists_for_repo(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let sessions = with_scenario_app_env(world, || {
        let backend = create_session_backend_or_local(world.repo_dir());
        backend.list_sessions()
    })
    .context("listing persisted Bitloops sessions")?;

    if let Some(session) = sessions
        .iter()
        .find(|session| session.agent_type == normalised_agent_name)
    {
        ensure!(
            !session.session_id.is_empty(),
            "expected {normalised_agent_name} session to have a session id"
        );
        ensure!(
            !session.transcript_path.is_empty(),
            "expected {normalised_agent_name} session to record a transcript path"
        );
        if !session.first_prompt.trim().is_empty() || session.pending.step_count > 0 {
            return Ok(());
        }
    }

    let expected_session_id = smoke_session_id(world, normalised_agent_name);
    let transcript_path = expected_smoke_transcript_path(world, normalised_agent_name);
    let context_paths = find_persisted_session_context_paths(world, &expected_session_id)
        .with_context(|| format!("locating persisted context for {expected_session_id}"))?;

    ensure!(
        !context_paths.is_empty(),
        "expected persisted {normalised_agent_name} session metadata for {expected_session_id}"
    );
    ensure!(
        transcript_path.exists(),
        "expected {normalised_agent_name} transcript at {}",
        transcript_path.display()
    );
    let transcript = fs::read_to_string(&transcript_path)
        .with_context(|| format!("reading {}", transcript_path.display()))?;
    ensure!(
        !transcript.trim().is_empty(),
        "expected {normalised_agent_name} transcript at {} to be non-empty",
        transcript_path.display()
    );

    let mut found_valid_context = false;
    for context_path in context_paths {
        let context = fs::read_to_string(&context_path)
            .with_context(|| format!("reading {}", context_path.display()))?;
        if context.contains(&format!("Session ID: {expected_session_id}"))
            && !context.trim().is_empty()
        {
            found_valid_context = true;
            break;
        }
    }

    ensure!(
        found_valid_context,
        "expected persisted {normalised_agent_name} context for session {expected_session_id}"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_exists_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mappings = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "Bitloops checkpoint mappings to be persisted",
        || {
            with_scenario_app_env(world, || read_commit_checkpoint_mappings(world.repo_dir()))
                .context("reading Bitloops checkpoint mappings")
        },
        |mappings| !mappings.is_empty(),
        |mappings| format!("mappings={}", mappings.len()),
    )?;
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

    let summary = with_scenario_app_env(world, || read_committed(world.repo_dir(), checkpoint_id))
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
    let mappings = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("at least {min_count} Bitloops checkpoint mappings"),
        || {
            with_scenario_app_env(world, || read_commit_checkpoint_mappings(world.repo_dir()))
                .context("reading Bitloops checkpoint mappings")
        },
        |mappings| mappings.len() >= min_count,
        |mappings| format!("mappings={}", mappings.len()),
    )?;
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
        build_git_command(world, &["log", "--pretty=format:%s|%aI", "-n", "30"], &[]),
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
    let count = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "DevQL artefacts query to return results",
        || count_artefacts_across_source_files(world),
        |count| *count >= 1,
        |count| format!("artefacts={count}"),
    )?;
    world.last_query_result_count = Some(count);
    ensure!(
        count >= 1,
        "expected at least 1 artefact from devql query, got {count}"
    );
    Ok(())
}

fn checkpoint_agent_candidates(agent: &str) -> Vec<String> {
    let mut candidates = vec![agent.to_string()];
    if agent == "claude" {
        candidates.push("claude-code".to_string());
    } else if agent == "claude-code" {
        candidates.push("claude".to_string());
    }
    candidates
}

fn build_chat_history_query(path: &str) -> String {
    format!(
        r#"repo("bitloops")->file("{}")->artefacts()->chatHistory()->limit(10)"#,
        escape_devql_string(path)
    )
}

fn count_chat_history_edges_for_agent(value: &serde_json::Value, agent_name: &str) -> usize {
    let candidates = checkpoint_agent_candidates(agent_name);
    value.as_array().map_or(0, |rows| {
        rows.iter()
            .flat_map(|row| {
                row.get("chatHistory")
                    .and_then(|chat_history| chat_history.get("edges"))
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|edge| {
                edge.get("node")
                    .and_then(|node| node.get("agent"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|candidate| candidates.iter().any(|value| value == candidate))
            })
            .count()
    })
}

pub fn assert_devql_checkpoints_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
    agent: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut max_count = 0_usize;
    for candidate in checkpoint_agent_candidates(agent) {
        let query = format!(
            r#"repo("bitloops")->checkpoints(agent:"{}")->limit(5)"#,
            escape_devql_string(&candidate)
        );
        let value = run_devql_query(world, &query)?;
        let count = count_json_array_rows(&value);
        max_count = max_count.max(count);
        if count >= 1 {
            world.last_query_result_count = Some(count);
            return Ok(());
        }
    }

    world.last_query_result_count = Some(max_count);
    ensure!(
        max_count >= 1,
        "expected at least 1 checkpoint for agent {agent}, got {max_count}"
    );
    Ok(())
}

pub fn assert_devql_chat_history_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let agent_name = world
        .agent_name
        .as_deref()
        .ok_or_else(|| anyhow!("no agent name captured for chat history assertion"))?;
    let agent_name = agent_name.to_string();
    let candidate_paths = chat_history_candidate_paths(world)?;
    let mut best_path = String::new();
    let mut best_count = 0_usize;

    for target_path in candidate_paths {
        let query = build_chat_history_query(&target_path);
        let value = run_devql_query(world, &query)?;
        let count = count_chat_history_edges_for_agent(&value, &agent_name);
        if count > best_count {
            best_count = count;
            best_path = target_path.clone();
        }
        if count >= 1 {
            world.last_query_result_count = Some(count);
            return Ok(());
        }
    }

    world.last_query_result_count = Some(best_count);
    ensure!(
        best_count >= 1,
        "expected at least 1 chat history result for agent `{agent_name}` across queryable touched paths, best path `{best_path}` produced {best_count}"
    );
    Ok(())
}

// ── DevQL ingest DB-first assertions and git topology helpers ───────────────

/// Parse a numeric field from the ingest summary output.
/// Format: "DevQL ingest complete: commits_processed=1, ...".
pub fn parse_ingest_summary_field(stdout: &str, field: &str) -> Option<usize> {
    let needle = format!("{field}=");
    let suffix = stdout.split(&needle).nth(1)?;
    let raw = suffix
        .split([',', '\n', ' '])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    raw.parse::<usize>().ok()
}

fn relational_db_path_for_world(world: &QatWorld) -> Result<std::path::PathBuf> {
    with_scenario_app_env(world, || {
        let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
            .context("resolving store backend config for ingest DB assertions")?;
        resolve_sqlite_db_path_for_repo(world.repo_dir(), cfg.relational.sqlite_path.as_deref())
            .context("resolving relational store path for ingest DB assertions")
    })
}

fn open_relational_connection(world: &QatWorld) -> Result<rusqlite::Connection> {
    let relational_db_path = relational_db_path_for_world(world)?;
    rusqlite::Connection::open(&relational_db_path).with_context(|| {
        format!(
            "opening relational store for ingest assertions at {}",
            relational_db_path.display()
        )
    })
}

fn query_repo_id_optional(conn: &rusqlite::Connection, sql: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    match conn
        .query_row(sql, [], |row| row.get::<_, String>(0))
        .optional()
    {
        Ok(value) => Ok(value),
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table") || message.contains("no such column") {
                return Ok(None);
            }
            Err(err).with_context(|| format!("querying repo id with `{sql}`"))
        }
    }
}

fn resolve_repo_id(conn: &rusqlite::Connection) -> Result<String> {
    for sql in [
        "SELECT repo_id FROM commit_ingest_ledger ORDER BY updated_at DESC LIMIT 1",
        "SELECT repo_id FROM commits ORDER BY committed_at DESC LIMIT 1",
        "SELECT repo_id FROM artefacts_historical LIMIT 1",
        "SELECT repo_id FROM symbol_features LIMIT 1",
        "SELECT repo_id FROM symbol_semantics LIMIT 1",
        "SELECT repo_id FROM symbol_embeddings LIMIT 1",
        "SELECT repo_id FROM artefacts_current LIMIT 1",
        "SELECT repo_id FROM current_file_state LIMIT 1",
        "SELECT repo_id FROM repositories WHERE provider = 'local' ORDER BY created_at DESC LIMIT 1",
        "SELECT repo_id FROM repositories ORDER BY created_at DESC LIMIT 1",
    ] {
        if let Some(repo_id) = query_repo_id_optional(conn, sql)? {
            return Ok(repo_id);
        }
    }
    bail!("unable to resolve repo_id from relational store for ingest assertions")
}

fn checkpoint_touched_paths_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Vec<String>> {
    let sql = "SELECT COALESCE(path_after, path_before) \
               FROM checkpoint_files \
               WHERE repo_id = ?1 \
               ORDER BY event_time DESC, checkpoint_id DESC, relation_id DESC \
               LIMIT 20";
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table") || message.contains("no such column") {
                return Ok(Vec::new());
            }
            return Err(err).with_context(|| format!("preparing `{sql}`"));
        }
    };
    let rows = stmt
        .query_map([repo_id], |row| row.get::<_, Option<String>>(0))
        .with_context(|| format!("querying checkpoint touched paths for repo `{repo_id}`"))?;
    let mut paths = Vec::new();
    for row in rows {
        let Some(path) = row.context("reading checkpoint touched path")? else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() || paths.iter().any(|existing| existing == path) {
            continue;
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn chat_history_candidate_paths(world: &QatWorld) -> Result<Vec<String>> {
    let mut candidates = vec![smoke_target_relative_path(world)];
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    for path in checkpoint_touched_paths_for_repo(&conn, &repo_id)? {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    }
    Ok(candidates)
}

fn git_reachable_shas(world: &QatWorld, max_count: Option<usize>) -> Result<Vec<String>> {
    let mut args_owned = vec!["rev-list".to_string()];
    if let Some(limit) = max_count {
        args_owned.push(format!("--max-count={limit}"));
    }
    args_owned.push("HEAD".to_string());
    let args: Vec<&str> = args_owned.iter().map(String::as_str).collect();
    let output = run_command_capture(
        world,
        "git rev-list HEAD",
        build_git_command(world, &args, &[]),
    )?;
    ensure_success(&output, "git rev-list HEAD")?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn completed_ledger_shas(world: &QatWorld) -> Result<Vec<String>> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT commit_sha \
             FROM commit_ingest_ledger \
             WHERE repo_id = ?1 AND history_status = 'completed'",
        )
        .context("preparing completed ledger SHA query")?;
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| row.get::<_, String>(0))
        .context("querying completed ledger SHAs")?;
    let mut shas = Vec::new();
    for row in rows {
        shas.push(row.context("reading completed ledger SHA row")?);
    }
    Ok(shas)
}

fn completed_ledger_count(world: &QatWorld) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_ingest_ledger \
             WHERE repo_id = ?1 AND history_status = 'completed'",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .context("counting completed commit_ingest_ledger rows")?;
    usize::try_from(count).context("converting completed ledger count to usize")
}

fn artefacts_current_count(world: &QatWorld) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .context("counting artefacts_current rows")?;
    usize::try_from(count).context("converting artefacts_current row count to usize")
}

fn artefacts_current_count_for_path(world: &QatWorld, path: &str) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id, path],
            |row| row.get(0),
        )
        .with_context(|| format!("counting artefacts_current rows for path `{path}`"))?;
    usize::try_from(count).context("converting artefacts_current path count to usize")
}

fn file_state_count_for_commit(world: &QatWorld, commit_sha: &str) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![repo_id, commit_sha],
            |row| row.get(0),
        )
        .with_context(|| format!("counting file_state rows for commit `{commit_sha}`"))?;
    usize::try_from(count).context("converting file_state count to usize")
}

fn commit_has_changed_files(world: &QatWorld, commit_sha: &str) -> Result<bool> {
    let has_parent = run_command_capture(
        world,
        "git rev-parse <sha>^",
        build_git_command(world, &["rev-parse", &format!("{commit_sha}^")], &[]),
    )?
    .status
    .success();

    let command_label = if has_parent {
        "git diff-tree --no-commit-id --name-only -r <sha>"
    } else {
        "git show --name-only --pretty=format: <sha>"
    };

    let command_args: Vec<String> = if has_parent {
        vec![
            "diff-tree".to_string(),
            "--no-commit-id".to_string(),
            "--name-only".to_string(),
            "-r".to_string(),
            commit_sha.to_string(),
        ]
    } else {
        vec![
            "show".to_string(),
            "--name-only".to_string(),
            "--pretty=format:".to_string(),
            commit_sha.to_string(),
        ]
    };
    let command_args_ref: Vec<&str> = command_args.iter().map(String::as_str).collect();
    let output = run_command_capture(
        world,
        command_label,
        build_git_command(world, &command_args_ref, &[]),
    )?;
    ensure_success(&output, command_label)?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty()))
}

fn current_branch_name(world: &QatWorld) -> Result<String> {
    let output = run_command_capture(
        world,
        "git rev-parse --abbrev-ref HEAD",
        build_git_command(world, &["rev-parse", "--abbrev-ref", "HEAD"], &[]),
    )?;
    ensure_success(&output, "git rev-parse --abbrev-ref HEAD")?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ensure!(!branch.is_empty(), "current git branch name is empty");
    Ok(branch)
}

fn post_commit_devql_refresh_disabled_env() -> [(&'static str, OsString); 1] {
    [(
        "BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH",
        OsString::from("1"),
    )]
}

fn write_and_commit_rust_file(
    world: &mut QatWorld,
    relative_path: &str,
    function_name: &str,
    body_value: usize,
    env: &[(&str, OsString)],
    commit_message: &str,
) -> Result<String> {
    let path = world.repo_dir().join(relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path `{relative_path}` has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    fs::write(
        &path,
        format!("pub fn {function_name}() -> usize {{\n    {body_value}\n}}\n"),
    )
    .with_context(|| format!("writing {}", path.display()))?;
    run_git_success(world, &["add", "-A"], env, "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", commit_message],
        env,
        "git commit ingest topology",
    )?;
    capture_head_sha(world)
}

fn set_expected_commits_and_paths(world: &mut QatWorld, shas: Vec<String>, paths: Vec<String>) {
    world.expected_commit_shas = shas;
    world.expected_paths = paths;
}

fn refresh_rewrite_delta(world: &mut QatWorld, expected_segment_len: usize) -> Result<()> {
    let post = git_reachable_shas(world, Some(expected_segment_len))?;
    world.post_rewrite_shas = post.clone();
    let pre: std::collections::BTreeSet<String> = world.pre_rewrite_shas.iter().cloned().collect();
    world.rewrite_new_shas = post.into_iter().filter(|sha| !pre.contains(sha)).collect();
    Ok(())
}

pub fn snapshot_ingest_db_state_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let shas = completed_ledger_shas(world)?;
    world.completed_ledger_count_snapshot = Some(shas.len());
    world.completed_ledger_shas_snapshot = shas;
    world.artefacts_current_count_snapshot = Some(artefacts_current_count(world)?);
    Ok(())
}

pub fn create_ingest_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "commit batch size must be greater than zero");
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let mut shas = Vec::with_capacity(count);
    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let seq = world.captured_commit_shas.len() + 1;
        let relative_path = format!("src/ingest_batch_{seq}.rs");
        let function_name = format!("ingest_batch_{seq}");
        let commit_message = format!("feat: ingest batch commit {}", index + 1);
        let sha = write_and_commit_rust_file(
            world,
            &relative_path,
            &function_name,
            seq,
            &disable_refresh_env,
            &commit_message,
        )?;
        shas.push(sha);
        paths.push(relative_path);
    }
    set_expected_commits_and_paths(world, shas, paths);
    Ok(())
}

pub fn create_non_ff_merge_with_two_feature_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let feature_branch = "qat-non-ff-feature";
    run_git_success(
        world,
        &["checkout", "-b", feature_branch],
        &[],
        "git checkout -b non-ff feature branch",
    )?;

    let first_path = "src/non_ff_feature_one.rs".to_string();
    let first_sha = write_and_commit_rust_file(
        world,
        &first_path,
        "non_ff_feature_one",
        1,
        &disable_refresh_env,
        "feat: non-ff feature commit 1",
    )?;
    let second_path = "src/non_ff_feature_two.rs".to_string();
    let second_sha = write_and_commit_rust_file(
        world,
        &second_path,
        "non_ff_feature_two",
        2,
        &disable_refresh_env,
        "feat: non-ff feature commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for non-ff merge",
    )?;
    run_git_success(
        world,
        &[
            "merge",
            "--no-ff",
            feature_branch,
            "-m",
            "merge: non-ff feature branch",
        ],
        &disable_refresh_env,
        "git merge --no-ff",
    )?;
    let merge_sha = capture_head_sha(world)?;
    set_expected_commits_and_paths(
        world,
        vec![first_sha, second_sha, merge_sha],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn create_ff_merge_with_two_feature_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let feature_branch = "qat-ff-feature";
    run_git_success(
        world,
        &["checkout", "-b", feature_branch],
        &[],
        "git checkout -b ff feature branch",
    )?;

    let first_path = "src/ff_feature_one.rs".to_string();
    let first_sha = write_and_commit_rust_file(
        world,
        &first_path,
        "ff_feature_one",
        11,
        &disable_refresh_env,
        "feat: ff feature commit 1",
    )?;
    let second_path = "src/ff_feature_two.rs".to_string();
    let second_sha = write_and_commit_rust_file(
        world,
        &second_path,
        "ff_feature_two",
        22,
        &disable_refresh_env,
        "feat: ff feature commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for ff merge",
    )?;
    run_git_success(
        world,
        &["merge", "--ff-only", feature_branch],
        &disable_refresh_env,
        "git merge --ff-only",
    )?;
    let _ = capture_head_sha(world)?;
    set_expected_commits_and_paths(
        world,
        vec![first_sha, second_sha],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn cherry_pick_two_commits_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let source_branch = "qat-cherry-source";
    run_git_success(
        world,
        &["checkout", "-b", source_branch],
        &[],
        "git checkout -b cherry-pick source",
    )?;

    let first_path = "src/cherry_source_one.rs".to_string();
    let source_sha_one = write_and_commit_rust_file(
        world,
        &first_path,
        "cherry_source_one",
        101,
        &disable_refresh_env,
        "feat: cherry source commit 1",
    )?;
    let second_path = "src/cherry_source_two.rs".to_string();
    let source_sha_two = write_and_commit_rust_file(
        world,
        &second_path,
        "cherry_source_two",
        202,
        &disable_refresh_env,
        "feat: cherry source commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for cherry-pick",
    )?;
    run_git_success(
        world,
        &["cherry-pick", source_sha_one.as_str()],
        &disable_refresh_env,
        "git cherry-pick source commit 1",
    )?;
    let cherry_sha_one = capture_head_sha(world)?;
    run_git_success(
        world,
        &["cherry-pick", source_sha_two.as_str()],
        &disable_refresh_env,
        "git cherry-pick source commit 2",
    )?;
    let cherry_sha_two = capture_head_sha(world)?;
    run_git_success(
        world,
        &["branch", "-D", source_branch],
        &[],
        "git branch -D cherry-pick source",
    )?;

    set_expected_commits_and_paths(
        world,
        vec![cherry_sha_one, cherry_sha_two],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn capture_top_reachable_shas_before_rewrite_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "rewrite capture count must be greater than zero");
    let pre = git_reachable_shas(world, Some(count))?;
    ensure!(
        pre.len() == count,
        "expected to capture {count} pre-rewrite SHAs, got {}",
        pre.len()
    );
    world.pre_rewrite_shas = pre;
    world.post_rewrite_shas.clear();
    world.rewrite_new_shas.clear();
    Ok(())
}

pub fn rewrite_last_commits_with_rebase_edit_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "rebase rewrite count must be greater than zero");
    ensure!(
        world.pre_rewrite_shas.len() == count,
        "pre-rewrite SHAs must be captured for exactly {count} commits before rebase rewrite"
    );
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let script = "echo '// qat rebase edit marker' >> src/main.rs && git add src/main.rs && git commit --amend --no-edit";
    let upstream = format!("HEAD~{count}");
    run_git_success(
        world,
        &["rebase", "-x", script, upstream.as_str()],
        &disable_refresh_env,
        "git rebase -x amend",
    )?;
    let _ = capture_head_sha(world)?;
    refresh_rewrite_delta(world, count)?;
    world.expected_commit_shas = world.rewrite_new_shas.clone();
    Ok(())
}

pub fn reset_and_rewrite_last_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "reset rewrite count must be greater than zero");
    ensure!(
        world.pre_rewrite_shas.len() == count,
        "pre-rewrite SHAs must be captured for exactly {count} commits before reset rewrite"
    );
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let target = format!("HEAD~{count}");
    run_git_success(
        world,
        &["reset", "--hard", target.as_str()],
        &[],
        "git reset --hard for rewrite",
    )?;

    let mut replacement_shas = Vec::with_capacity(count);
    let mut replacement_paths = Vec::with_capacity(count);
    for index in 0..count {
        let seq = world.captured_commit_shas.len() + 1;
        let relative_path = format!("src/reset_rewrite_{seq}.rs");
        let function_name = format!("reset_rewrite_{seq}");
        let sha = write_and_commit_rust_file(
            world,
            &relative_path,
            &function_name,
            500 + index,
            &disable_refresh_env,
            &format!("feat: reset rewrite replacement {}", index + 1),
        )?;
        replacement_shas.push(sha);
        replacement_paths.push(relative_path);
    }
    set_expected_commits_and_paths(world, replacement_shas, replacement_paths);
    refresh_rewrite_delta(world, count)?;
    Ok(())
}

pub fn assert_all_reachable_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let reachable = git_reachable_shas(world, None)?;
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = reachable
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "reachable commits missing completed ledger rows: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_artefacts_current_has_rows(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = artefacts_current_count(world)?;
    ensure!(
        count > 0,
        "expected artefacts_current to contain rows, got {count}"
    );
    Ok(())
}

pub fn assert_artefacts_current_contains_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = artefacts_current_count_for_path(world, path)?;
    ensure!(
        count > 0,
        "expected artefacts_current rows for `{path}`, got {count}"
    );
    Ok(())
}

pub fn assert_expected_shas_completed_in_ledger(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for this scenario"
    );
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = world
        .expected_commit_shas
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "expected commit SHAs missing from completed ledger: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_expected_shas_have_file_state_rows(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for file_state assertion"
    );
    let mut missing = Vec::new();
    for sha in &world.expected_commit_shas {
        if file_state_count_for_commit(world, sha)? == 0 && commit_has_changed_files(world, sha)? {
            missing.push(sha.clone());
        }
    }
    ensure!(
        missing.is_empty(),
        "expected file_state rows for commit SHAs, but none found for: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_no_new_completed_shas_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before: std::collections::BTreeSet<String> = world
        .completed_ledger_shas_snapshot
        .iter()
        .cloned()
        .collect();
    let after: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let new_shas: Vec<String> = after.difference(&before).cloned().collect();
    ensure!(
        new_shas.is_empty(),
        "expected no new completed ledger SHAs, got: {}",
        new_shas.join(", ")
    );
    Ok(())
}

pub fn assert_exact_expected_shas_newly_completed_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for new-ledger assertion"
    );
    let before: std::collections::BTreeSet<String> = world
        .completed_ledger_shas_snapshot
        .iter()
        .cloned()
        .collect();
    let after: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let actual_new: std::collections::BTreeSet<String> =
        after.difference(&before).cloned().collect();
    let expected_new: std::collections::BTreeSet<String> =
        world.expected_commit_shas.iter().cloned().collect();
    ensure!(
        actual_new == expected_new,
        "expected newly completed SHAs {:?}, got {:?}",
        expected_new,
        actual_new
    );
    Ok(())
}

pub fn assert_ledger_completed_count_unchanged_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .completed_ledger_count_snapshot
        .ok_or_else(|| anyhow!("completed ledger count snapshot is missing"))?;
    let after = completed_ledger_count(world)?;
    ensure!(
        before == after,
        "expected completed ledger count to stay {before}, got {after}"
    );
    Ok(())
}

pub fn assert_artefacts_current_count_unchanged_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .artefacts_current_count_snapshot
        .ok_or_else(|| anyhow!("artefacts_current count snapshot is missing"))?;
    let after = artefacts_current_count(world)?;
    ensure!(
        before == after,
        "expected artefacts_current count to stay {before}, got {after}"
    );
    Ok(())
}

pub fn assert_artefacts_current_count_increased_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .artefacts_current_count_snapshot
        .ok_or_else(|| anyhow!("artefacts_current count snapshot is missing"))?;
    let after = artefacts_current_count(world)?;
    ensure!(
        after > before,
        "expected artefacts_current count to increase from {before}, got {after}"
    );
    Ok(())
}

pub fn assert_only_latest_reachable_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
    latest_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        latest_count > 0,
        "latest reachable SHA count must be greater than zero"
    );
    let reachable = git_reachable_shas(world, None)?;
    ensure!(
        reachable.len() >= latest_count,
        "expected at least {latest_count} reachable commits, found {}",
        reachable.len()
    );
    let expected_latest: std::collections::BTreeSet<String> =
        reachable.iter().take(latest_count).cloned().collect();
    let completed_set: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let reachable_set: std::collections::BTreeSet<String> = reachable.into_iter().collect();
    let completed_reachable: std::collections::BTreeSet<String> = completed_set
        .intersection(&reachable_set)
        .cloned()
        .collect();
    ensure!(
        completed_reachable == expected_latest,
        "expected completed reachable SHAs {:?}, got {:?}",
        expected_latest,
        completed_reachable
    );
    Ok(())
}

pub fn assert_rewrite_new_shas_count(
    world: &QatWorld,
    repo_name: &str,
    expected_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        world.rewrite_new_shas.len() == expected_count,
        "expected {expected_count} rewritten SHAs, got {} (new={:?}, pre={:?}, post={:?})",
        world.rewrite_new_shas.len(),
        world.rewrite_new_shas,
        world.pre_rewrite_shas,
        world.post_rewrite_shas
    );
    Ok(())
}

pub fn assert_pre_rewrite_shas_absent_from_post_segment(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.pre_rewrite_shas.is_empty() && !world.post_rewrite_shas.is_empty(),
        "pre/post rewrite SHA segments are not populated"
    );
    let post: std::collections::BTreeSet<String> =
        world.post_rewrite_shas.iter().cloned().collect();
    let retained_old: Vec<String> = world
        .pre_rewrite_shas
        .iter()
        .filter(|sha| post.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        retained_old.is_empty(),
        "expected rewritten old SHAs to be absent from post-rewrite segment, retained: {}",
        retained_old.join(", ")
    );
    Ok(())
}

pub fn assert_rewrite_new_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.rewrite_new_shas.is_empty(),
        "rewrite_new_shas is empty; rewrite assertions require rewritten SHAs"
    );
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = world
        .rewrite_new_shas
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "rewritten SHAs missing from completed ledger rows: {}",
        missing.join(", ")
    );
    Ok(())
}
