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

fn recursive_symbol_file_paths(world: &QatWorld, stem: &str) -> Vec<String> {
    let src_root = world.repo_dir().join("src");
    if !src_root.exists() {
        return Vec::new();
    }

    let target_file_names = [format!("{stem}.ts"), format!("{stem}.tsx")];
    let mut matches = walkdir::WalkDir::new(&src_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_str()?;
            target_file_names
                .contains(&file_name.to_string())
                .then_some(path.to_path_buf())
        })
        .filter_map(|path| {
            path.strip_prefix(world.repo_dir())
                .ok()
                .map(Path::to_path_buf)
        })
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    matches.sort();
    matches
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
        for path in recursive_symbol_file_paths(world, &stem) {
            push_if_exists(path);
        }
    } else {
        let stem = to_kebab_case(symbol_alias);
        push_if_exists("src/new-caller.ts".to_string());
        push_if_exists(format!("src/{stem}.ts"));
        for path in recursive_symbol_file_paths(world, &stem) {
            push_if_exists(path);
        }
        push_if_exists("src/index.ts".to_string());
    }

    candidates
}

fn candidate_symbol_fqns(world: &QatWorld, symbol_alias: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut push_unique = |candidate: String| {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    if let Some((class_name, method_name)) = symbol_alias.split_once('.') {
        for path in candidate_symbol_file_paths(world, symbol_alias) {
            push_unique(format!("{path}::{class_name}::{method_name}"));
            push_unique(format!("{path}::{method_name}"));
        }
    } else {
        for path in candidate_symbol_file_paths(world, symbol_alias) {
            push_unique(format!("{path}::{symbol_alias}"));
        }
    }

    candidates
}

fn resolve_symbol_fqn_alias(world: &mut QatWorld, symbol_alias: &str) -> Result<String> {
    if symbol_alias.contains("::") {
        return Ok(symbol_alias.to_string());
    }

    let candidate_fqns = candidate_symbol_fqns(world, symbol_alias);
    if let Some(candidate) = candidate_fqns.first() {
        return Ok(candidate.clone());
    }

    let mut suffixes = Vec::new();
    if let Some((class_name, method_name)) = symbol_alias.split_once('.') {
        suffixes.push(format!("::{class_name}::{method_name}"));
        suffixes.push(format!("::{method_name}"));
    } else {
        suffixes.push(format!("::{symbol_alias}"));
    }

    let value = run_devql_graphql_query(
        world,
        r#"query {
  artefacts(first: 2000) {
    edges {
      node {
        path
        symbolFqn
      }
    }
  }
}"#,
    )?;
    let rows = value
        .get("artefacts")
        .and_then(|artefacts| artefacts.get("edges"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected artefact edges in GraphQL response"))?;

    let candidate_paths = candidate_symbol_file_paths(world, symbol_alias);
    for file_path in &candidate_paths {
        for suffix in &suffixes {
            if let Some(symbol_fqn) = rows.iter().find_map(|row| {
                let node = row.get("node")?;
                let path = node.get("path").and_then(serde_json::Value::as_str)?;
                let symbol_fqn = node.get("symbolFqn").and_then(serde_json::Value::as_str)?;
                (path == file_path && symbol_fqn.ends_with(suffix)).then_some(symbol_fqn)
            }) {
                return Ok(symbol_fqn.to_string());
            }
        }
    }

    for suffix in &suffixes {
        if let Some(symbol_fqn) = rows.iter().find_map(|row| {
            row.get("node")
                .and_then(|node| node.get("symbolFqn"))
                .and_then(serde_json::Value::as_str)
                .filter(|candidate| candidate.ends_with(suffix))
        }) {
            return Ok(symbol_fqn.to_string());
        }
    }

    bail!("unable to resolve symbol alias `{symbol_alias}` to a DevQL symbolFqn")
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

fn text_has_missing_production_artefacts_error(text: &str) -> bool {
    let combined = text.to_ascii_lowercase();
    combined.contains("no production artefacts found for commit")
        || combined.contains("materialize production artefacts first")
}

fn run_claude_code_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    ensure_claude_authenticated(world)?;
    if claude_fallback_marker_exists(world) {
        let file_path = apply_smoke_prompt_edit(world, prompt)?;
        simulate_claude_session_for_prompt(world, prompt, &file_path)?;
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

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    simulate_claude_session_for_prompt(world, prompt, &file_path)?;
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

fn run_deterministic_claude_smoke_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    simulate_claude_session_for_prompt(world, prompt, &file_path)
}

fn run_cursor_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_CURSOR);
    let transcript_path = smoke_transcript_path(world, AGENT_NAME_CURSOR, "jsonl");
    let model = "gpt-5.4-mini";

    let payload = serde_json::json!({
        "conversation_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "prompt": prompt,
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "cursor", "before-submit-prompt"],
        "bitloops hooks cursor before-submit-prompt",
        &payload,
    )?;

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    append_cursor_transcript_turn(
        &transcript_path,
        prompt,
        &smoke_response_text(prompt),
        &file_path,
    )?;

    let stop_payload = serde_json::json!({
        "conversation_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "cursor", "stop"],
        "bitloops hooks cursor stop",
        &stop_payload,
    )
}

fn run_gemini_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_GEMINI);
    let transcript_path = smoke_transcript_path(world, AGENT_NAME_GEMINI, "json");
    let model = "gemini-2.5-pro";

    if !transcript_path.exists() {
        let session_start_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path.display().to_string(),
            "modelSlug": model,
        })
        .to_string();
        run_bitloops_with_stdin(
            world,
            &["hooks", "gemini", "session-start"],
            "bitloops hooks gemini session-start",
            &session_start_payload,
        )?;
    }

    let before_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "prompt": prompt,
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "gemini", "before-agent"],
        "bitloops hooks gemini before-agent",
        &before_payload,
    )?;

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    append_gemini_transcript_turn(
        &transcript_path,
        prompt,
        &smoke_response_text(prompt),
        &file_path,
    )?;

    let after_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "gemini", "after-agent"],
        "bitloops hooks gemini after-agent",
        &after_payload,
    )
}

fn run_copilot_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_COPILOT);
    let transcript_path = copilot_transcript_path(world, &session_id);
    let model = "gpt-5.4";

    if !transcript_path.exists() {
        let session_start_payload = serde_json::json!({
            "sessionId": session_id,
            "initialPrompt": prompt,
            "modelSlug": model,
        })
        .to_string();
        run_bitloops_with_stdin(
            world,
            &["hooks", "copilot", "session-start"],
            "bitloops hooks copilot session-start",
            &session_start_payload,
        )?;
    }

    let prompt_payload = serde_json::json!({
        "sessionId": session_id,
        "prompt": prompt,
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "copilot", "user-prompt-submitted"],
        "bitloops hooks copilot user-prompt-submitted",
        &prompt_payload,
    )?;

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    append_copilot_transcript_turn(
        &transcript_path,
        prompt,
        &smoke_response_text(prompt),
        &file_path,
    )?;

    let stop_payload = serde_json::json!({
        "sessionId": session_id,
        "transcriptPath": transcript_path.display().to_string(),
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "copilot", "agent-stop"],
        "bitloops hooks copilot agent-stop",
        &stop_payload,
    )
}

fn run_codex_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_CODEX);
    let transcript_path = smoke_transcript_path(world, AGENT_NAME_CODEX, "jsonl");
    let model = "gpt-5.4-codex";

    if !transcript_path.exists() {
        let session_start_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path.display().to_string(),
            "modelSlug": model,
        })
        .to_string();
        run_bitloops_with_stdin(
            world,
            &["hooks", "codex", "session-start"],
            "bitloops hooks codex session-start",
            &session_start_payload,
        )?;
    }

    let prompt_submit_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "prompt": prompt,
        "model": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "codex", "user-prompt-submit"],
        "bitloops hooks codex user-prompt-submit",
        &prompt_submit_payload,
    )?;

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    append_codex_transcript_turn(
        &transcript_path,
        prompt,
        &smoke_response_text(prompt),
        &file_path,
    )?;

    let stop_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "modelSlug": model,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "codex", "stop"],
        "bitloops hooks codex stop",
        &stop_payload,
    )
}

fn run_opencode_prompt(world: &QatWorld, prompt: &str) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_OPEN_CODE);
    let transcript_path = smoke_transcript_path(world, AGENT_NAME_OPEN_CODE, "jsonl");

    if !transcript_path.exists() {
        let session_start_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path.display().to_string(),
        })
        .to_string();
        run_bitloops_with_stdin(
            world,
            &["hooks", "opencode", "session-start"],
            "bitloops hooks opencode session-start",
            &session_start_payload,
        )?;
    }

    let turn_start_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
        "prompt": prompt,
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "opencode", "turn-start"],
        "bitloops hooks opencode turn-start",
        &turn_start_payload,
    )?;

    let file_path = apply_smoke_prompt_edit(world, prompt)?;
    append_opencode_transcript_turn(
        &transcript_path,
        prompt,
        &smoke_response_text(prompt),
        &file_path,
    )?;

    let turn_end_payload = serde_json::json!({
        "session_id": session_id,
        "transcript_path": transcript_path.display().to_string(),
    })
    .to_string();
    run_bitloops_with_stdin(
        world,
        &["hooks", "opencode", "turn-end"],
        "bitloops hooks opencode turn-end",
        &turn_end_payload,
    )
}

fn smoke_session_id(world: &QatWorld, agent_name: &str) -> String {
    let run_slug = world
        .run_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "qat-run".to_string());
    format!("{agent_name}-{run_slug}")
}

fn smoke_transcript_path(
    world: &QatWorld,
    agent_name: &str,
    extension: &str,
) -> std::path::PathBuf {
    let session_id = smoke_session_id(world, agent_name);
    world
        .run_dir()
        .join("agent-sessions")
        .join(agent_name)
        .join(format!("{session_id}.{extension}"))
}

fn copilot_transcript_path(world: &QatWorld, session_id: &str) -> std::path::PathBuf {
    world
        .run_dir()
        .join("home")
        .join(".copilot")
        .join("session-state")
        .join(session_id)
        .join("events.jsonl")
}

fn expected_smoke_transcript_path(world: &QatWorld, agent_name: &str) -> std::path::PathBuf {
    let session_id = smoke_session_id(world, agent_name);
    match agent_name {
        AGENT_NAME_COPILOT => copilot_transcript_path(world, &session_id),
        AGENT_NAME_GEMINI => smoke_transcript_path(world, agent_name, "json"),
        _ => smoke_transcript_path(world, agent_name, "jsonl"),
    }
}

fn find_persisted_session_context_paths(
    world: &QatWorld,
    session_id: &str,
) -> Result<Vec<std::path::PathBuf>> {
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
    let runtime_root = resolved.blob_store_path.join("runtime");
    if !runtime_root.exists() {
        return Ok(Vec::new());
    }

    let session_marker = format!("{}/", session_id);
    let mut pending = vec![runtime_root];
    let mut matches = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) != Some("context.md") {
                continue;
            }
            let path_text = path.to_string_lossy();
            if path_text.contains("session-metadata") && path_text.contains(&session_marker) {
                matches.push(path);
            }
        }
    }
    matches.sort();
    Ok(matches)
}

fn smoke_target_relative_path(world: &QatWorld) -> String {
    let app_path = world.repo_dir().join("my-app").join("src").join("App.tsx");
    if app_path.exists() {
        "my-app/src/App.tsx".to_string()
    } else if world.repo_dir().join("src").join("lib.rs").exists() {
        "src/lib.rs".to_string()
    } else if world
        .repo_dir()
        .join("src")
        .join("services")
        .join("user-service.ts")
        .exists()
    {
        "src/services/user-service.ts".to_string()
    } else {
        ".qat-claude-fallback-change.txt".to_string()
    }
}

fn smoke_response_text(prompt: &str) -> String {
    if prompt == FIRST_CLAUDE_PROMPT {
        "Replaced the Vite example with a simple hello world page.".to_string()
    } else if prompt == SECOND_CLAUDE_PROMPT {
        "Changed the hello world color to blue.".to_string()
    } else {
        "Applied the requested change.".to_string()
    }
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}

fn append_jsonl_line(path: &Path, value: &serde_json::Value) -> Result<()> {
    ensure_parent_dir(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(value).context("serializing jsonl line")?
    )
    .with_context(|| format!("writing {}", path.display()))
}

fn count_non_empty_lines(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count())
}

fn append_cursor_transcript_turn(
    path: &Path,
    prompt: &str,
    response: &str,
    file_path: &str,
) -> Result<()> {
    append_jsonl_line(
        path,
        &serde_json::json!({
            "type": "user",
            "message": {
                "content": [{"type": "text", "text": prompt}]
            }
        }),
    )?;
    append_jsonl_line(
        path,
        &serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": response},
                    {"type": "text", "text": format!("Modified {file_path}")}
                ]
            }
        }),
    )
}

fn append_gemini_transcript_turn(
    path: &Path,
    prompt: &str,
    response: &str,
    file_path: &str,
) -> Result<()> {
    let mut messages = read_gemini_messages(path)?;
    let user_index = messages.len() + 1;
    let tool_name = if prompt == FIRST_CLAUDE_PROMPT {
        "write_file"
    } else {
        "edit_file"
    };

    messages.push(serde_json::json!({
        "id": format!("msg-{user_index}"),
        "type": "user",
        "content": prompt,
    }));
    messages.push(serde_json::json!({
        "id": format!("msg-{}", user_index + 1),
        "type": "gemini",
        "content": response,
        "toolCalls": [{
            "id": format!("tool-{user_index}"),
            "name": tool_name,
            "args": { "file_path": file_path },
            "status": "completed"
        }]
    }));

    ensure_parent_dir(path)?;
    fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({ "messages": messages }))
            .context("serializing gemini transcript")?,
    )
    .with_context(|| format!("writing {}", path.display()))
}

fn read_gemini_messages(path: &Path) -> Result<Vec<serde_json::Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if data.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(Vec::new());
    }

    let parsed: serde_json::Value =
        serde_json::from_slice(&data).context("parsing gemini transcript json")?;
    Ok(parsed
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn append_copilot_transcript_turn(
    path: &Path,
    prompt: &str,
    response: &str,
    file_path: &str,
) -> Result<()> {
    append_jsonl_line(
        path,
        &serde_json::json!({
            "type": "user.message",
            "data": {
                "content": prompt
            }
        }),
    )?;
    append_jsonl_line(
        path,
        &serde_json::json!({
            "type": "tool.execution_complete",
            "data": {
                "model": "gpt-5.4",
                "toolTelemetry": {
                    "properties": {
                        "filePaths": serde_json::to_string(&vec![file_path])
                            .context("serializing copilot file paths")?
                    }
                }
            }
        }),
    )?;
    append_jsonl_line(
        path,
        &serde_json::json!({
            "type": "assistant.message",
            "data": {
                "content": response,
                "outputTokens": 42
            }
        }),
    )
}

fn append_codex_transcript_turn(
    path: &Path,
    prompt: &str,
    response: &str,
    file_path: &str,
) -> Result<()> {
    append_jsonl_line(
        path,
        &serde_json::json!({
            "id": format!("msg-{}", count_non_empty_lines(path)? + 1),
            "role": "user",
            "content": prompt,
        }),
    )?;
    append_jsonl_line(
        path,
        &serde_json::json!({
            "id": format!("msg-{}", count_non_empty_lines(path)? + 1),
            "role": "assistant",
            "content": response,
            "file_path": file_path,
        }),
    )
}

fn append_opencode_transcript_turn(
    path: &Path,
    prompt: &str,
    response: &str,
    file_path: &str,
) -> Result<()> {
    let line_count = count_non_empty_lines(path)?;
    let user_index = line_count + 1;
    let created_at = 1_708_300_000_i64 + line_count as i64 * 5;
    let assistant_index = user_index + 1;

    append_jsonl_line(
        path,
        &serde_json::json!({
            "id": format!("msg-{user_index}"),
            "role": "user",
            "content": prompt,
            "time": {
                "created": created_at
            }
        }),
    )?;
    append_jsonl_line(
        path,
        &serde_json::json!({
            "id": format!("msg-{assistant_index}"),
            "role": "assistant",
            "content": response,
            "time": {
                "created": created_at + 1,
                "completed": created_at + 2
            },
            "tokens": {
                "input": 128,
                "output": 64,
                "reasoning": 8,
                "cache": {
                    "read": 4,
                    "write": 12
                }
            },
            "cost": 0.001,
            "parts": [
                {
                    "type": "text",
                    "text": response
                },
                {
                    "type": "tool",
                    "tool": if prompt == FIRST_CLAUDE_PROMPT { "write" } else { "edit" },
                    "callID": format!("call-{assistant_index}"),
                    "state": {
                        "status": "completed",
                        "input": {
                            "file_path": file_path
                        },
                        "output": "Applied edit"
                    }
                }
            ]
        }),
    )
}

fn apply_smoke_prompt_edit(world: &QatWorld, prompt: &str) -> Result<String> {
    let relative_path = smoke_target_relative_path(world);
    let full_path = world.repo_dir().join(&relative_path);
    if relative_path == ".qat-claude-fallback-change.txt" {
        let next = if prompt == SECOND_CLAUDE_PROMPT {
            "color=blue\n"
        } else {
            "hello=bitloops\n"
        };
        fs::write(&full_path, next).with_context(|| format!("writing {}", full_path.display()))?;
        return Ok(relative_path);
    }

    let current = fs::read_to_string(&full_path)
        .with_context(|| format!("reading {}", full_path.display()))?;
    if relative_path == "src/lib.rs" {
        let next = if prompt.to_ascii_lowercase().contains("subtract function") {
            if current.contains("pub fn subtract(") {
                current
            } else {
                format!(
                    "{current}\n\npub fn subtract(a: i32, b: i32) -> i32 {{\n    a - b\n}}\n\n#[cfg(test)]\nmod subtract_tests {{\n    use super::*;\n\n    #[test]\n    fn test_subtract() {{\n        assert_eq!(subtract(7, 4), 3);\n    }}\n}}\n"
                )
            }
        } else if prompt.to_ascii_lowercase().contains("divide function") {
            if current.contains("pub fn divide(") {
                current
            } else {
                format!(
                    "{current}\n\npub fn divide(a: i32, b: i32) -> i32 {{\n    a / b\n}}\n\n#[cfg(test)]\nmod divide_tests {{\n    use super::*;\n\n    #[test]\n    fn test_divide() {{\n        assert_eq!(divide(8, 2), 4);\n    }}\n}}\n"
                )
            }
        } else if prompt.to_ascii_lowercase().contains("modulo function") {
            if current.contains("pub fn modulo(") {
                current
            } else {
                format!(
                    "{current}\n\npub fn modulo(a: i32, b: i32) -> i32 {{\n    a % b\n}}\n\n#[cfg(test)]\nmod modulo_tests {{\n    use super::*;\n\n    #[test]\n    fn test_modulo() {{\n        assert_eq!(modulo(9, 4), 1);\n    }}\n}}\n"
                )
            }
        } else {
            current
        };
        fs::write(&full_path, next).with_context(|| format!("writing {}", full_path.display()))?;
        return Ok(relative_path);
    }

    if relative_path == "src/services/user-service.ts" {
        let next = if prompt == FIRST_CLAUDE_PROMPT {
            if current.contains("const normalizedName = name.trim();")
                && current.contains("normalizedName.toUpperCase()")
            {
                current
            } else {
                current.replace(
                    "    return { id: crypto.randomUUID(), name: name.trim() };",
                    "    const normalizedName = name.trim();\n    return { id: crypto.randomUUID(), name: normalizedName.toUpperCase() };",
                )
            }
        } else if prompt == SECOND_CLAUDE_PROMPT {
            if current.contains("normalizedName.toLowerCase()") {
                current
            } else if current.contains("normalizedName.toUpperCase()") {
                current.replace(
                    "normalizedName.toUpperCase()",
                    "normalizedName.toLowerCase()",
                )
            } else if current.contains("name: name.trim()") {
                current.replace(
                    "    return { id: crypto.randomUUID(), name: name.trim() };",
                    "    const normalizedName = name.trim();\n    return { id: crypto.randomUUID(), name: normalizedName.toLowerCase() };",
                )
            } else {
                current
            }
        } else {
            current
        };
        fs::write(&full_path, next).with_context(|| format!("writing {}", full_path.display()))?;
        return Ok(relative_path);
    }

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

    fs::write(&full_path, next).with_context(|| format!("writing {}", full_path.display()))?;
    Ok(relative_path)
}

fn simulate_claude_session_for_prompt(
    world: &QatWorld,
    prompt: &str,
    file_path: &str,
) -> Result<()> {
    let session_id = smoke_session_id(world, AGENT_NAME_CLAUDE_CODE);
    let transcript_path = smoke_transcript_path(world, AGENT_NAME_CLAUDE_CODE, "jsonl");
    append_jsonl_line(
        &transcript_path,
        &serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": prompt }]
            }
        }),
    )?;
    append_jsonl_line(
        &transcript_path,
        &serde_json::json!({
            "type": "assistant",
            "uuid": format!("assistant-{}", count_non_empty_lines(&transcript_path)? + 1),
            "message": {
                "model": "claude-opus-4-1",
                "content": [
                    {
                        "type": "text",
                        "text": smoke_response_text(prompt)
                    },
                    {
                        "type": "tool_use",
                        "name": if prompt == FIRST_CLAUDE_PROMPT { "Write" } else { "Edit" },
                        "input": {
                            "file_path": file_path
                        }
                    }
                ]
            }
        }),
    )?;

    if count_non_empty_lines(&transcript_path)? == 2 {
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
    }

    let prompt_payload = serde_json::json!({
        "session_id": session_id.clone(),
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
        .env(bitloops::cli::versioncheck::DISABLE_VERSION_CHECK_ENV, "1")
        .env("BITLOOPS_DEVQL_EMBEDDING_PROVIDER", "disabled")
        .env("BITLOOPS_DEVQL_SEMANTIC_PROVIDER", "disabled")
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");
    if !world.watcher_autostart_enabled {
        command.env(
            bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV,
            "1",
        );
    } else {
        command.env_remove(bitloops::host::devql::watch::DISABLE_WATCHER_AUTOSTART_ENV);
    }
    if let Some(timeout_secs) = world.watcher_idle_timeout_secs {
        command.env(
            bitloops::host::devql::watch::WATCHER_IDLE_TIMEOUT_ENV,
            timeout_secs.to_string(),
        );
    } else {
        command.env_remove(bitloops::host::devql::watch::WATCHER_IDLE_TIMEOUT_ENV);
    }
    Ok(command)
}

fn build_git_command(world: &QatWorld, args: &[&str], env: &[(&str, OsString)]) -> Command {
    let mut command = Command::new("git");
    if args == ["add", "-A"] {
        command
            .args(["add", "-A", "--", ".", ":(exclude).bitloops/stores"])
            .current_dir(world.repo_dir());
    } else {
        command.args(args).current_dir(world.repo_dir());
    }

    // Set HOME and XDG dirs so that git hooks (post-commit, post-checkout,
    // post-merge) invoked by this git process resolve daemon state paths to
    // the scenario-isolated directory instead of the system HOME.  Without
    // this, hook-triggered sync tasks are routed to the system daemon and
    // can race against the scenario daemon on the same SQLite database.
    let run_dir = world.run_dir();
    let home_dir = run_dir.join("home");
    command
        .env("HOME", &home_dir)
        .env("USERPROFILE", &home_dir)
        .env("XDG_STATE_HOME", home_dir.join("xdg-state"));

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
