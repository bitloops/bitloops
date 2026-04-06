fn escape_devql_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn to_kebab_case(input: &str) -> String {
    let mut output = String::new();
    for (index, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            output.push('-');
        }
        output.push(ch.to_ascii_lowercase());
    }
    output
}

fn candidate_symbol_file_paths(world: &QatWorld, symbol_alias: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut push_if_exists = |path: String| {
        if world.repo_dir().join(&path).exists() && !candidates.contains(&path) {
            candidates.push(path);
        }
    };

    if let Some((class_name, _)) = symbol_alias.split_once('.') {
        let stem = to_kebab_case(class_name);
        for path in [
            format!("src/services/{stem}.ts"),
            format!("src/controllers/{stem}.ts"),
            format!("src/repository/{stem}.ts"),
            format!("src/models/{stem}.ts"),
            format!("src/{stem}.ts"),
        ] {
            push_if_exists(path);
        }
    } else {
        let stem = to_kebab_case(symbol_alias);
        for path in [
            "src/new-caller.ts".to_string(),
            format!("src/{stem}.ts"),
            "src/index.ts".to_string(),
        ] {
            push_if_exists(path);
        }
    }

    candidates
}

fn resolve_symbol_fqn_alias(world: &mut QatWorld, symbol_alias: &str) -> Result<String> {
    if symbol_alias.contains("::") {
        return Ok(symbol_alias.to_string());
    }

    let mut suffixes = Vec::new();
    if let Some((class_name, method_name)) = symbol_alias.split_once('.') {
        suffixes.push(format!("::{class_name}::{method_name}"));
        suffixes.push(format!("::{method_name}"));
    } else {
        suffixes.push(format!("::{symbol_alias}"));
    }

    for file_path in candidate_symbol_file_paths(world, symbol_alias) {
        let query = format!(
            r#"repo("bitloops")->file("{}")->artefacts()->limit(500)"#,
            escape_devql_string(&file_path)
        );
        let Ok(value) = run_devql_query(world, &query) else {
            continue;
        };
        let Some(rows) = value.as_array() else {
            continue;
        };
        for suffix in &suffixes {
            if let Some(symbol_fqn) = rows.iter().find_map(|row| {
                row.get("symbolFqn")
                    .and_then(serde_json::Value::as_str)
                    .filter(|candidate| candidate.ends_with(suffix))
            }) {
                return Ok(symbol_fqn.to_string());
            }
        }
    }

    Ok(symbol_alias.to_string())
}

fn resolve_file_path_for_symbol(world: &mut QatWorld, symbol_fqn: &str) -> Result<String> {
    if let Some((path, _)) = symbol_fqn.split_once("::")
        && !path.is_empty()
    {
        return Ok(path.to_string());
    }

    if world.repo_dir().join(symbol_fqn).exists() {
        return Ok(symbol_fqn.to_string());
    }

    if let Some(path) = candidate_symbol_file_paths(world, symbol_fqn)
        .into_iter()
        .next()
    {
        return Ok(path);
    }

    bail!("could not resolve file path for symbol `{symbol_fqn}`")
}

fn resolve_file_path_for_symbol_alias(world: &mut QatWorld, symbol_alias: &str) -> Result<String> {
    if let Some(path) = candidate_symbol_file_paths(world, symbol_alias)
        .into_iter()
        .next()
    {
        return Ok(path);
    }
    let symbol_fqn = resolve_symbol_fqn_alias(world, symbol_alias)?;
    resolve_file_path_for_symbol(world, &symbol_fqn)
}

fn parse_last_command_stdout_json(world: &QatWorld) -> Result<serde_json::Value> {
    let stdout = world
        .last_command_stdout
        .as_deref()
        .ok_or_else(|| anyhow!("no command stdout captured"))?;
    serde_json::from_str(stdout.trim()).context("parsing captured command json output")
}

fn count_testlens_payload_rows(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(arr) => arr.len(),
        serde_json::Value::Object(obj) => {
            if let Some(arr) = obj
                .get("covering_tests")
                .and_then(serde_json::Value::as_array)
            {
                return arr.len();
            }
            if obj
                .get("coverage")
                .is_some_and(|coverage| !coverage.is_null())
            {
                return 1;
            }
            if obj.get("summary").is_some() {
                return 1;
            }
            0
        }
        _ => 0,
    }
}

fn parse_knowledge_item_id_from_output(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix("knowledge item:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn resolve_knowledge_ref_from_input(world: &QatWorld, input: &str) -> Result<String> {
    if input.starts_with("knowledge:") {
        return Ok(input.to_string());
    }

    let knowledge_item_id = world
        .knowledge_items_by_url
        .get(input)
        .ok_or_else(|| anyhow!("no knowledge item id captured for URL `{input}`"))?;
    Ok(format!("knowledge:{knowledge_item_id}"))
}

fn parse_knowledge_versions_count(stdout: &str) -> Result<usize> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        return Ok(count_json_array_rows(&value));
    }

    for line in stdout.lines() {
        if let Some(raw) = line.trim().strip_prefix("versions:") {
            return raw
                .trim()
                .parse::<usize>()
                .with_context(|| format!("parsing versions count from `{raw}`"));
        }
    }

    bail!("unable to parse knowledge versions count from command output")
}

fn is_knowledge_provider_config_missing(stderr: &str) -> bool {
    stderr.contains("knowledge.providers.github")
        || stderr.contains("knowledge.providers.jira")
        || stderr.contains("knowledge.providers.confluence")
        || stderr.contains("knowledge.providers.atlassian")
}

fn is_knowledge_item_not_found(stderr: &str) -> bool {
    stderr.contains("knowledge item `") && stderr.contains("not found")
}

fn knowledge_fallback_active(world: &QatWorld) -> bool {
    world.run_dir().join(KNOWLEDGE_FALLBACK_MARKER).exists()
}

fn activate_knowledge_fallback(world: &mut QatWorld, url: &str, with_commit: bool) -> Result<()> {
    let knowledge_item_id = world
        .knowledge_items_by_url
        .get(url)
        .cloned()
        .unwrap_or_else(|| format!("qat-knowledge-{}", world.knowledge_items_by_url.len() + 1));
    world
        .knowledge_items_by_url
        .insert(url.to_string(), knowledge_item_id.clone());
    world
        .knowledge_versions_by_ref
        .entry(knowledge_item_id.clone())
        .or_insert(1);
    world.last_knowledge_add_had_commit_association = Some(with_commit);
    world.last_command_exit_code = Some(0);
    world.last_command_stdout = Some(format!("knowledge item: {knowledge_item_id}\n"));

    fs::write(world.run_dir().join(KNOWLEDGE_FALLBACK_MARKER), b"1").with_context(|| {
        format!(
            "writing knowledge fallback marker in {}",
            world.run_dir().display()
        )
    })?;
    Ok(())
}

fn synthetic_knowledge_rows(world: &QatWorld) -> Vec<serde_json::Value> {
    let mut urls: Vec<&String> = world.knowledge_items_by_url.keys().collect();
    urls.sort();

    urls.into_iter()
        .filter_map(|url| {
            let knowledge_item_id = world.knowledge_items_by_url.get(url)?;
            let (provider, source_kind) = if url.contains("github.com") && url.contains("/issues/")
            {
                ("github", "issue")
            } else if url.contains("github.com") && url.contains("/pull/") {
                ("github", "pull_request")
            } else {
                ("unknown", "unknown")
            };
            Some(serde_json::json!({
                "knowledgeItemId": knowledge_item_id,
                "sourceUrl": url,
                "provider": provider,
                "sourceKind": source_kind
            }))
        })
        .collect()
}

fn fallback_knowledge_versions_count(world: &QatWorld, knowledge_ref: &str) -> usize {
    knowledge_ref
        .strip_prefix("knowledge:")
        .and_then(|knowledge_item_id| world.knowledge_versions_by_ref.get(knowledge_item_id))
        .copied()
        .unwrap_or(1)
}

/// Parse a key=value field from sync validation output.
/// Format: "artefacts: expected=2 actual=0 missing=2 stale=0 mismatched=0"
pub fn parse_validation_field(stdout: &str, field: &str) -> Option<usize> {
    let needle = format!("{field}=");
    for line in stdout.lines() {
        if let Some(pos) = line.find(&needle) {
            let after = &line[pos + needle.len()..];
            let raw = after
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .filter(|v| !v.is_empty())?;
            return raw.parse::<usize>().ok();
        }
    }
    None
}

fn extract_ingest_metric(stdout: &str, key: &str) -> Option<u64> {
    let suffix = stdout.split(key).nth(1)?;
    let raw = suffix
        .split([',', '\n', ' '])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    raw.parse::<u64>().ok()
}

fn claude_fallback_marker_exists(world: &QatWorld) -> bool {
    world.run_dir().join(CLAUDE_FALLBACK_MARKER).exists()
}

fn claude_fallback_enabled() -> bool {
    std::env::var("BITLOOPS_QAT_DISABLE_CLAUDE_FALLBACK")
        .map(|value| value != "1")
        .unwrap_or(true)
}

fn activate_claude_fallback(world: &QatWorld, reason: &str) -> Result<()> {
    append_world_log(world, &format!("Claude fallback activated: {reason}\n"))?;
    fs::write(world.run_dir().join(CLAUDE_FALLBACK_MARKER), b"1")
        .with_context(|| format!("writing fallback marker in {}", world.run_dir().display()))
}

fn semantic_clones_fallback_active(world: &QatWorld) -> bool {
    world.semantic_clones_fallback_active
        || world
            .run_dir()
            .join(SEMANTIC_CLONES_FALLBACK_MARKER)
            .exists()
}

fn repo_has_head(world: &QatWorld) -> Result<bool> {
    let output = run_command_capture(
        world,
        "git rev-parse HEAD",
        build_git_command(world, &["rev-parse", "--verify", "HEAD"], &[]),
    )?;
    Ok(output.status.success())
}

fn configure_git_identity(world: &QatWorld) -> Result<()> {
    let commands = [
        ["config", "user.name", "Bitloops QAT"],
        ["config", "user.email", "bitloops-qat@example.com"],
        ["config", "commit.gpgsign", "false"],
    ];

    for args in commands {
        run_git_success(world, &args, &[], "git config")?;
    }
    Ok(())
}

fn ensure_claude_authenticated(world: &QatWorld) -> Result<()> {
    if claude_fallback_marker_exists(world) {
        return Ok(());
    }
    match claude_auth_status_logged_in(world) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            if claude_fallback_enabled() {
                activate_claude_fallback(
                    world,
                    &format!("Claude auth status check failed: {err}"),
                )?;
                return Ok(());
            }
            return Err(err);
        }
    }

    let login_command = std::env::var(CLAUDE_AUTH_LOGIN_COMMAND_ENV)
        .unwrap_or_else(|_| DEFAULT_CLAUDE_AUTH_LOGIN_COMMAND.to_string());
    let login_timeout = resolve_claude_auth_timeout();
    let login_output = run_command_capture_with_timeout(
        world,
        "claude auth login",
        build_host_shell_command(world, &login_command)?,
        login_timeout,
    )
    .context("running Claude auth login")?;
    if !login_output.status.success() {
        if claude_fallback_enabled() {
            let stdout = String::from_utf8_lossy(&login_output.stdout);
            let stderr = String::from_utf8_lossy(&login_output.stderr);
            activate_claude_fallback(
                world,
                &format!("Claude auth login failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"),
            )?;
            return Ok(());
        }
        ensure_success(&login_output, "claude auth login")?;
    }

    match claude_auth_status_logged_in(world) {
        Ok(true) => {}
        Ok(false) => {
            if claude_fallback_enabled() {
                activate_claude_fallback(
                    world,
                    "Claude auth login completed but Claude is still not authenticated.",
                )?;
                return Ok(());
            }
            bail!("Claude auth login completed but Claude is still not authenticated");
        }
        Err(err) => {
            if claude_fallback_enabled() {
                activate_claude_fallback(
                    world,
                    &format!("Claude auth verification failed after login: {err}"),
                )?;
                return Ok(());
            }
            return Err(err);
        }
    }
    Ok(())
}

fn claude_auth_status_logged_in(world: &QatWorld) -> Result<bool> {
    let status_command = std::env::var(CLAUDE_AUTH_STATUS_COMMAND_ENV)
        .unwrap_or_else(|_| DEFAULT_CLAUDE_AUTH_STATUS_COMMAND.to_string());
    let status_timeout = resolve_claude_auth_timeout();
    let status_output = run_command_capture_with_timeout(
        world,
        "claude auth status",
        build_host_shell_command(world, &status_command)?,
        status_timeout,
    )
    .context("running Claude auth status")?;

    let stdout = String::from_utf8_lossy(&status_output.stdout).to_string();
    parse_claude_auth_logged_in(&stdout).ok_or_else(|| {
        anyhow!(
            "unable to parse Claude auth status output as JSON boolean `loggedIn`\nstdout:\n{}",
            stdout
        )
    })
}

fn parse_claude_auth_logged_in(stdout: &str) -> Option<bool> {
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|value| value.get("loggedIn").and_then(serde_json::Value::as_bool))
}

fn output_has_claude_auth_failure(output: &Output) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    text_has_claude_auth_failure(&combined)
}

fn text_has_claude_auth_failure(text: &str) -> bool {
    let combined = text.to_ascii_lowercase();
    combined.contains("not logged in")
        || combined.contains("run /login")
        || combined.contains("authentication required")
}

fn run_claude_code_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    ensure_claude_authenticated(world)?;
    if claude_fallback_marker_exists(world) {
        apply_claude_prompt_fallback_edit(world, prompt)?;
        simulate_claude_session_for_prompt(world, prompt)?;
        return Ok(());
    }

    let command_spec = std::env::var("BITLOOPS_QAT_CLAUDE_CMD")
        .unwrap_or_else(|_| DEFAULT_CLAUDE_CODE_COMMAND.to_string());
    let timeout = resolve_claude_timeout();
    let output = run_claude_command_capture(
        world,
        &format!("{command_spec} {}", shell_single_quote(prompt)),
        timeout,
    )
    .context("running external Claude Code prompt")?;
    if output.status.success() {
        return Ok(());
    }

    if output_has_claude_auth_failure(&output) {
        return ensure_success(&output, "claude prompt");
    }

    let fallback_enabled = claude_fallback_enabled();
    if !fallback_enabled {
        return ensure_success(&output, "claude prompt");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    append_world_log(
        world,
        &format!(
            "Claude command failed; activating QAT fallback.\nstdout:\n{stdout}\nstderr:\n{stderr}\n"
        ),
    )?;

    apply_claude_prompt_fallback_edit(world, prompt)?;
    simulate_claude_session_for_prompt(world, prompt)?;
    fs::write(world.run_dir().join(CLAUDE_FALLBACK_MARKER), b"1")
        .with_context(|| format!("writing fallback marker in {}", world.run_dir().display()))?;
    Ok(())
}

fn run_claude_command_capture(
    world: &QatWorld,
    script: &str,
    timeout: StdDuration,
) -> Result<Output> {
    run_command_capture_with_timeout(
        world,
        "claude prompt",
        build_host_shell_command(world, script)?,
        timeout,
    )
}

fn resolve_claude_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(CLAUDE_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_CLAUDE_TIMEOUT_SECS,
    )
}

fn resolve_claude_auth_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(CLAUDE_AUTH_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_CLAUDE_AUTH_TIMEOUT_SECS,
    )
}

fn resolve_command_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(COMMAND_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_COMMAND_TIMEOUT_SECS,
    )
}

fn parse_timeout_seconds(raw: Option<&str>, default_secs: u64) -> StdDuration {
    let seconds = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_secs);
    StdDuration::from_secs(seconds)
}

fn run_bitloops_success(world: &QatWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    ensure_success(&output, label)
}

fn run_bitloops_with_stdin(
    world: &QatWorld,
    args: &[&str],
    label: &str,
    stdin_payload: &str,
) -> Result<()> {
    let output = run_command_capture_with_stdin(
        world,
        label,
        build_bitloops_command(world, args)?,
        stdin_payload,
    )
    .with_context(|| format!("running {label}"))?;
    ensure_success(&output, label)
}

fn apply_claude_prompt_fallback_edit(world: &QatWorld, prompt: &str) -> Result<()> {
    let app_path = world.repo_dir().join("my-app").join("src").join("App.tsx");
    if !app_path.exists() {
        let fallback_path = world.repo_dir().join(".qat-claude-fallback-change.txt");
        let next = if prompt == SECOND_CLAUDE_PROMPT {
            "color=blue\n"
        } else {
            "hello=bitloops\n"
        };
        fs::write(&fallback_path, next)
            .with_context(|| format!("writing {}", fallback_path.display()))?;
        return Ok(());
    }

    let current =
        fs::read_to_string(&app_path).with_context(|| format!("reading {}", app_path.display()))?;
    let next = if prompt == FIRST_CLAUDE_PROMPT {
        "export function App() {\n  return <h1>Hello Bitloops</h1>;\n}\n".to_string()
    } else if prompt == SECOND_CLAUDE_PROMPT {
        if current.contains("style={{ color: 'blue' }}") {
            current
        } else if current.contains("<h1>Hello Bitloops</h1>") {
            current.replace(
                "<h1>Hello Bitloops</h1>",
                "<h1 style={{ color: 'blue' }}>Hello Bitloops</h1>",
            )
        } else if current.contains("<h1>Hello Vite</h1>") {
            current.replace(
                "<h1>Hello Vite</h1>",
                "<h1 style={{ color: 'blue' }}>Hello Bitloops</h1>",
            )
        } else {
            "export function App() {\n  return <h1 style={{ color: 'blue' }}>Hello Bitloops</h1>;\n}\n".to_string()
        }
    } else {
        current
    };

    fs::write(&app_path, next).with_context(|| format!("writing {}", app_path.display()))?;
    Ok(())
}

fn simulate_claude_session_for_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = format!("qat-session-{}", short_run_id());
    let transcript_path = world.run_dir().join(format!("{session_id}.jsonl"));
    let transcript_line =
        serde_json::json!({ "role": "user", "content": prompt, "agent": "claude-code" });
    fs::write(
        &transcript_path,
        format!(
            "{}\n",
            serde_json::to_string(&transcript_line)
                .context("serializing fallback transcript line")?
        ),
    )
    .with_context(|| format!("writing {}", transcript_path.display()))?;

    let session_start_payload = serde_json::json!({
        "session_id": session_id.clone(),
        "transcript_path": transcript_path.display().to_string()
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "claude-code", "session-start"],
        "bitloops hooks claude-code session-start",
        &session_start_payload,
    )?;

    let prompt_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "prompt": prompt
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "claude-code", "user-prompt-submit"],
        "bitloops hooks claude-code user-prompt-submit",
        &prompt_payload,
    )?;

    let stop_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string()
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "claude-code", "stop"],
        "bitloops hooks claude-code stop",
        &stop_payload,
    )?;
    Ok(())
}

fn build_host_shell_command(world: &QatWorld, script: &str) -> Result<Command> {
    let mut command = Command::new("bash");
    command
        .args(["-lc", script])
        .current_dir(world.repo_dir())
        .env("PWD", world.repo_dir())
        .env("ACCESSIBLE", "1")
        .env("BITLOOPS_QAT_ACTIVE", "1");
    Ok(command)
}

fn run_git_success(
    world: &QatWorld,
    args: &[&str],
    env: &[(&str, OsString)],
    label: &str,
) -> Result<()> {
    let output = run_command_capture(world, label, build_git_command(world, args, env))?;
    ensure_success(&output, label)
}

fn build_bitloops_command(world: &QatWorld, args: &[&str]) -> Result<Command> {
    let run_dir = world.run_dir();
    let home_dir = run_dir.join("home");
    let xdg_config_home = home_dir.join("xdg");
    let xdg_state_home = home_dir.join("xdg-state");
    let xdg_cache_home = home_dir.join("xdg-cache");
    let xdg_data_home = home_dir.join("xdg-data");
    for dir in [
        &xdg_config_home,
        &xdg_state_home,
        &xdg_cache_home,
        &xdg_data_home,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let mut command = Command::new(&world.run_config().binary_path);
    command
        .args(args)
        .current_dir(world.repo_dir())
        .env("HOME", &home_dir)
        .env("USERPROFILE", &home_dir)
        .env("XDG_CONFIG_HOME", &xdg_config_home)
        .env("XDG_STATE_HOME", &xdg_state_home)
        .env("XDG_CACHE_HOME", &xdg_cache_home)
        .env("XDG_DATA_HOME", &xdg_data_home)
        .env("ACCESSIBLE", "1")
        .env("BITLOOPS_QAT_ACTIVE", "1")
        .env("BITLOOPS_TEST_TTY", "0")
        .env("BITLOOPS_DEVQL_EMBEDDING_PROVIDER", "disabled")
        .env("BITLOOPS_DEVQL_SEMANTIC_PROVIDER", "disabled")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
    Ok(command)
}

fn build_git_command(world: &QatWorld, args: &[&str], env: &[(&str, OsString)]) -> Command {
    let mut command = Command::new("git");
    command.args(args).current_dir(world.repo_dir());

    if let Some(binary_dir) = world.run_config().binary_path.parent() {
        let mut paths = Vec::new();
        paths.push(binary_dir.to_path_buf());
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("PATH", joined);
        }
    }

    for (key, value) in env {
        command.env(key, value);
    }
    command
}

fn run_command_capture(world: &QatWorld, label: &str, command: Command) -> Result<Output> {
    run_command_capture_with_timeout(world, label, command, resolve_command_timeout())
}

fn run_command_capture_with_timeout(
    world: &QatWorld,
    label: &str,
    mut command: Command,
    timeout: StdDuration,
) -> Result<Output> {
    let command_debug = format!("{command:?}");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("executing {label}"))?;

    let started = Instant::now();
    loop {
        match child
            .try_wait()
            .with_context(|| format!("polling {label}"))?
        {
            Some(_) => {
                let output = child
                    .wait_with_output()
                    .with_context(|| format!("collecting output for {label}"))?;
                append_command_log(world, label, &command_debug, &output)?;
                return Ok(output);
            }
            None => {
                if started.elapsed() >= timeout {
                    append_world_log(
                        world,
                        &format!(
                            "{label} timed out after {}s; terminating process.\n",
                            timeout.as_secs()
                        ),
                    )?;
                    if let Err(err) = child.kill() {
                        append_world_log(
                            world,
                            &format!("failed to terminate timed out {label}: {err}\n"),
                        )?;
                    }
                    let output = child
                        .wait_with_output()
                        .with_context(|| format!("collecting timed out output for {label}"))?;
                    append_command_log(world, label, &command_debug, &output)?;
                    return Ok(output);
                }
                std::thread::sleep(StdDuration::from_millis(100));
            }
        }
    }
}

fn run_command_capture_with_stdin(
    world: &QatWorld,
    label: &str,
    mut command: Command,
    stdin_payload: &str,
) -> Result<Output> {
    let command_debug = format!("{command:?}");
    command.stdin(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning {label}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(stdin_payload.as_bytes())
            .with_context(|| format!("writing stdin payload for {label}"))?;
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("waiting for {label}"))?;
    append_command_log(world, label, &command_debug, &output)?;
    Ok(output)
}

fn ensure_success(output: &Output, label: &str) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn append_world_log(world: &QatWorld, message: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(world.terminal_log_path())
        .with_context(|| format!("opening {}", world.terminal_log_path().display()))?;
    file.write_all(message.as_bytes())
        .with_context(|| format!("writing {}", world.terminal_log_path().display()))
}

fn append_command_log(
    world: &QatWorld,
    label: &str,
    command_debug: &str,
    output: &Output,
) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(world.terminal_log_path())
        .with_context(|| format!("opening {}", world.terminal_log_path().display()))?;
    writeln!(file, "=== {label} ===")?;
    writeln!(file, "{command_debug}")?;
    writeln!(file, "status: {:?}", output.status)?;
    writeln!(file, "stdout:\n{}", String::from_utf8_lossy(&output.stdout))?;
    writeln!(file, "stderr:\n{}", String::from_utf8_lossy(&output.stderr))?;
    writeln!(file)?;
    Ok(())
}

fn write_run_metadata(world: &QatWorld) -> Result<()> {
    let metadata = RunMetadata {
        scenario_name: world
            .scenario_name
            .as_deref()
            .ok_or_else(|| anyhow!("scenario name missing"))?,
        scenario_slug: world
            .scenario_slug
            .as_deref()
            .ok_or_else(|| anyhow!("scenario slug missing"))?,
        flow_name: world
            .flow_name
            .as_deref()
            .ok_or_else(|| anyhow!("flow name missing"))?,
        run_dir: world.run_dir().display().to_string(),
        repo_dir: world.repo_dir().display().to_string(),
        terminal_log: world.terminal_log_path().display().to_string(),
        binary_path: world.run_config().binary_path.display().to_string(),
        created_at: now_rfc3339()?,
    };
    let payload = serde_json::to_vec_pretty(&metadata).context("serializing qat run metadata")?;
    fs::write(world.metadata_path(), payload)
        .with_context(|| format!("writing {}", world.metadata_path().display()))
}

fn create_offline_vite_react_ts_scaffold(repo_dir: &Path) -> Result<()> {
    let app_dir = repo_dir.join("my-app");
    let src_dir = app_dir.join("src");
    fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;

    fs::write(
        app_dir.join("package.json"),
        "{\n  \"name\": \"my-app\",\n  \"private\": true,\n  \"version\": \"0.0.0\",\n  \"type\": \"module\",\n  \"scripts\": {\n    \"dev\": \"vite\",\n    \"build\": \"vite build\",\n    \"preview\": \"vite preview\"\n  }\n}\n",
    )
    .context("writing package.json")?;
    fs::write(
        app_dir.join("index.html"),
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    <meta charset=\"UTF-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n    <title>Vite + React + TS</title>\n  </head>\n  <body>\n    <div id=\"root\"></div>\n    <script type=\"module\" src=\"/src/main.tsx\"></script>\n  </body>\n</html>\n",
    )
    .context("writing index.html")?;
    fs::write(
        src_dir.join("App.tsx"),
        "export function App() {\n  return <h1>Hello Vite</h1>;\n}\n",
    )
    .context("writing App.tsx")?;
    fs::write(
        src_dir.join("main.tsx"),
        "import React from 'react';\nimport ReactDOM from 'react-dom/client';\nimport { App } from './App';\n\nReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(\n  <React.StrictMode>\n    <App />\n  </React.StrictMode>\n);\n",
    )
    .context("writing main.tsx")?;
    Ok(())
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting current timestamp")
}

fn git_date_for_relative_day(days_ago: i64) -> Result<String> {
    let target_date = OffsetDateTime::now_utc().date() - Duration::days(days_ago);
    let timestamp = PrimitiveDateTime::new(target_date, Time::from_hms(12, 0, 0)?)
        .assume_offset(UtcOffset::UTC);
    timestamp
        .format(&Rfc3339)
        .context("formatting git author date")
}

fn expected_date_for_relative_day(days_ago: i64) -> Result<String> {
    let timestamp = git_date_for_relative_day(days_ago)?;
    Ok(timestamp.chars().take(10).collect())
}

fn short_run_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn shell_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', r#"'"'"'"#))
}
