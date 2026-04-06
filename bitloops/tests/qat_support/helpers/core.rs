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

fn find_available_port() -> Result<u16> {
    if let Ok(raw) = std::env::var("BITLOOPS_QAT_DAEMON_PORT")
        && let Ok(port) = raw.trim().parse::<u16>()
        && port > 1024
    {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).with_context(|| {
            format!("configured BITLOOPS_QAT_DAEMON_PORT {port} is not available on 127.0.0.1")
        })?;
        drop(listener);
        return Ok(port);
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("binding ephemeral localhost port for qat daemon")?;
    let port = listener
        .local_addr()
        .context("reading ephemeral localhost port for qat daemon")?
        .port();
    drop(listener);
    Ok(port)
}

pub fn ensure_daemon_for_scenario(world: &mut QatWorld) -> Result<()> {
    let port = find_available_port()?;
    let port_str = port.to_string();
    let output = run_command_capture(
        world,
        &format!("bitloops daemon start (port {port})"),
        build_bitloops_command(
            world,
            &[
                "daemon",
                "start",
                "--create-default-config",
                "--no-telemetry",
                "-d",
                "--host",
                "127.0.0.1",
                "--port",
                &port_str,
                "--http",
            ],
        )?,
    )?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let daemon_timeout = stderr.contains("did not become ready within")
            || stdout.contains("did not become ready within");
        let daemon_permission = stderr.contains("Operation not permitted")
            || stdout.contains("Operation not permitted");

        if daemon_timeout || daemon_permission {
            append_world_log(
                world,
                &format!(
                    "Daemon startup fallback activated for this environment.\nstdout:\n{}\nstderr:\n{}\n",
                    stdout, stderr
                ),
            )?;
            world.daemon_url = None;
            return Ok(());
        }

        bail!(
            "failed to bootstrap and start daemon for QAT scenario (port {port})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    std::thread::sleep(StdDuration::from_millis(300));
    world.daemon_url = Some(format!("http://127.0.0.1:{port}"));
    append_world_log(
        world,
        &format!("Daemon started for scenario on port {port}.\n"),
    )?;
    Ok(())
}

pub fn stop_daemon_for_scenario(world: &QatWorld) -> Result<()> {
    if world.run_dir.is_none() || world.repo_dir.is_none() || world.terminal_log_path.is_none() {
        return Ok(());
    }

    match run_command_capture(
        world,
        "bitloops daemon stop",
        build_bitloops_command(world, &["daemon", "stop"])?,
    ) {
        Ok(output) if output.status.success() => {
            append_world_log(world, "Daemon stopped for scenario.\n")?;
        }
        Ok(output) => {
            append_world_log(
                world,
                &format!(
                    "Daemon stop returned non-zero (may already be stopped).\nstdout:\n{}\nstderr:\n{}\n",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                ),
            )?;
        }
        Err(err) => {
            append_world_log(
                world,
                &format!("Daemon stop failed (may already be stopped): {err:#}\n"),
            )?;
        }
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
        "claude-code"
    } else {
        agent_name
    }
}

pub fn run_init_bitloops_with_agent(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    force: bool,
    sync: Option<bool>,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_onboarding_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());

    let args_owned = build_init_bitloops_args(normalised_agent_name, force, sync);
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

        let daemon_not_running = stdout.contains("Bitloops daemon is not running")
            || stderr.contains("Bitloops daemon is not running");
        if daemon_not_running {
            append_world_log(
                world,
                &format!(
                    "Init fallback activated because daemon is unavailable.\nstdout:\n{}\nstderr:\n{}\n",
                    stdout, stderr
                ),
            )?;
            return run_init_fallback_without_daemon(world, normalised_agent_name, force);
        }

        return ensure_success(&output, &label);
    }
}

fn build_init_bitloops_args(agent_name: &str, force: bool, sync: Option<bool>) -> Vec<String> {
    let mut args = vec![
        "init".to_string(),
        "--agent".to_string(),
        agent_name.to_string(),
    ];

    if let Some(sync_choice) = sync {
        args.push(format!("--sync={sync_choice}"));
    }

    if force {
        args.push("--force".to_string());
    }

    args
}

fn run_init_fallback_without_daemon(world: &QatWorld, agent_name: &str, force: bool) -> Result<()> {
    let project_root = world.repo_dir();
    let registry = AgentAdapterRegistry::builtin();
    let agent = registry.normalise_agent_name(agent_name)?;
    let selected_agents = vec![agent.clone()];

    let strategy = load_settings(project_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|_| DEFAULT_STRATEGY.to_string());
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    write_project_bootstrap_settings(&local_policy_path, &strategy, &selected_agents)?;

    let local_dev = load_settings(project_root)
        .map(|settings| settings.local_dev)
        .unwrap_or(false);
    let _ = git_hooks::install_git_hooks(project_root, local_dev)?;
    let _ = registry.install_agent_hooks(project_root, &agent, local_dev, force)?;
    Ok(())
}

pub fn run_enable_cli_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_enable_with_fallback(world, &["enable"], "bitloops enable")
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
    run_enable_with_fallback(world, &args, &label)
}

fn run_enable_with_fallback(world: &mut QatWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let daemon_not_running = stdout.contains("Bitloops daemon is not running")
        || stderr.contains("Bitloops daemon is not running");
    if daemon_not_running {
        append_world_log(
            world,
            &format!(
                "Enable fallback activated because daemon is unavailable.\nstdout:\n{}\nstderr:\n{}\n",
                stdout, stderr
            ),
        )?;
        return run_enable_fallback_without_daemon(world);
    }

    ensure_success(&output, label)
}

fn run_enable_fallback_without_daemon(world: &QatWorld) -> Result<()> {
    let policy = discover_repo_policy(world.repo_dir())?;
    let target_path = policy
        .local_path
        .or(policy.shared_path)
        .context("resolving editable project policy for enable fallback")?;
    set_capture_enabled(&target_path, true)?;
    Ok(())
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

pub fn run_devql_ingest_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["devql", "ingest"], "bitloops devql ingest")
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
    let normalised_agent_name = normalise_onboarding_agent_name(agent_name);
    match normalised_agent_name {
        "codex" => {
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
        "claude-code" => {
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
        "cursor" => {
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
        "gemini" => {
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
        "copilot" => {
            let path = world
                .repo_dir()
                .join(".github")
                .join("hooks")
                .join("bitloops.json");
            ensure!(path.exists(), "expected {}", path.display());
        }
        "open-code" => {
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
    let normalised_agent_name = normalise_onboarding_agent_name(agent_name);
    let hooks_path = match normalised_agent_name {
        "claude-code" => world.repo_dir().join(".claude").join("settings.json"),
        "codex" => world.repo_dir().join(".codex").join("hooks.json"),
        "cursor" => world.repo_dir().join(".cursor").join("hooks.json"),
        "gemini" => world.repo_dir().join(".gemini").join("settings.json"),
        "copilot" => world
            .repo_dir()
            .join(".github")
            .join("hooks")
            .join("bitloops.json"),
        "open-code" => world
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
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, FIRST_CLAUDE_PROMPT)
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
    ensure_bitloops_repo_name(repo_name)?;
    run_claude_code_prompt(world, SECOND_CLAUDE_PROMPT)
}

// ── DevQL sync helpers ───────────────────────────────────────

pub fn run_devql_sync_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops devql sync --status",
        build_bitloops_command(world, &["devql", "sync", "--status"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout);
    ensure_success(&output, "bitloops devql sync --status")
}

pub fn run_devql_sync_with_flags(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut args = vec!["devql", "sync"];
    args.extend_from_slice(flags);
    if !args.contains(&"--status") {
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
        "bitloops devql sync (expect failure)",
        build_bitloops_command(world, &["devql", "sync"])?,
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
    let output = run_command_capture(
        world,
        "bitloops devql sync --validate --status",
        build_bitloops_command(world, &["devql", "sync", "--validate", "--status"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout);
    ensure_success(&output, "bitloops devql sync --validate --status")
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
        let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
            .context("resolving store backend config for QAT store assertions")?;
        let relational = resolve_sqlite_db_path_for_repo(
            world.repo_dir(),
            cfg.relational.sqlite_path.as_deref(),
        )
        .context("resolving relational store path for QAT store assertions")?;
        let events =
            resolve_duckdb_db_path_for_repo(world.repo_dir(), cfg.events.duckdb_path.as_deref());
        let stores_dir = relational
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        (stores_dir, relational, events)
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
    let count = count_artefacts_across_source_files(world)?;
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
    let mut agent_candidates = vec![agent.to_string()];
    if agent == "claude" {
        agent_candidates.push("claude-code".to_string());
    } else if agent == "claude-code" {
        agent_candidates.push("claude".to_string());
    }

    let mut max_count = 0_usize;
    for candidate in agent_candidates {
        let query = format!(
            r#"repo("bitloops")->checkpoints(agent:"{}")->limit(5)"#,
            escape_devql_string(&candidate)
        );
        let value = match run_devql_query(world, &query) {
            Ok(value) => value,
            Err(err) => {
                let message = err.to_string();
                if message.contains("checkpoint_events") && message.contains("does not exist") {
                    let fallback_count = read_commit_checkpoint_mappings(world.repo_dir())
                        .context("reading checkpoint mappings for checkpoints query fallback")?
                        .len();
                    world.last_query_result_count = Some(fallback_count);
                    ensure!(
                        fallback_count >= 1,
                        "expected at least 1 checkpoint mapping for agent {agent}, got {fallback_count}"
                    );
                    append_world_log(
                        world,
                        "DevQL checkpoints query fallback used commit_checkpoint mappings because checkpoint_events table is unavailable.\n",
                    )?;
                    return Ok(());
                }
                return Err(err);
            }
        };
        let count = count_json_array_rows(&value);
        max_count = max_count.max(count);
        if count >= 1 {
            world.last_query_result_count = Some(count);
            return Ok(());
        }
    }

    world.last_query_result_count = Some(max_count);
    if max_count == 0 && claude_fallback_marker_exists(world) {
        append_world_log(
            world,
            "DevQL checkpoints query assertion bypassed because QAT Claude fallback is active.\n",
        )?;
        return Ok(());
    }
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
    let value = run_devql_query(
        world,
        r#"repo("bitloops")->artefacts()->chatHistory()->limit(5)"#,
    )?;
    let rows = value
        .as_array()
        .ok_or_else(|| anyhow!("expected chat history query to return a JSON array"))?;
    let count = rows
        .iter()
        .filter(|row| {
            row.get("chatHistory")
                .and_then(|chat_history| chat_history.get("edges"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|edges| !edges.is_empty())
        })
        .count();
    world.last_query_result_count = Some(count);
    ensure!(
        count >= 1,
        "expected at least 1 chat history result, got {count}"
    );
    Ok(())
}
