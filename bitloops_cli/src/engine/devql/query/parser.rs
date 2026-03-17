// Query filter types (defined here; used by query_parser, query_executor, deps_query).

#[derive(Debug, Clone, Default)]
struct ParsedDevqlQuery {
    repo: Option<String>,
    as_of: Option<AsOfSelector>,
    file: Option<String>,
    files_path: Option<String>,
    artefacts: ArtefactFilter,
    checkpoints: CheckpointFilter,
    telemetry: TelemetryFilter,
    deps: DepsFilter,
    has_artefacts_stage: bool,
    has_deps_stage: bool,
    has_checkpoints_stage: bool,
    has_telemetry_stage: bool,
    has_chat_history_stage: bool,
    limit: usize,
    select_fields: Vec<String>,
}

#[derive(Debug, Clone)]
enum AsOfSelector {
    Ref(String),
    Commit(String),
    SaveCurrent,
    SaveRevision(String),
}

#[derive(Debug, Clone, Default)]
struct ArtefactFilter {
    kind: Option<String>,
    symbol_fqn: Option<String>,
    lines: Option<(i32, i32)>,
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CheckpointFilter {
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct TelemetryFilter {
    event_type: Option<String>,
    agent: Option<String>,
    since: Option<String>,
}

#[derive(Debug, Clone)]
struct DepsFilter {
    kind: Option<String>,
    direction: String,
    include_unresolved: bool,
}

impl Default for DepsFilter {
    fn default() -> Self {
        Self {
            kind: None,
            direction: "out".to_string(),
            include_unresolved: true,
        }
    }
}

fn parse_devql_query(query: &str) -> Result<ParsedDevqlQuery> {
    let mut parsed = ParsedDevqlQuery {
        limit: 100,
        ..Default::default()
    };

    let stages = query
        .split("->")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    if stages.is_empty() {
        bail!("empty DevQL query")
    }

    for stage in stages {
        if let Some(inner) = stage
            .strip_prefix("repo(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.repo = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("asOf(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            if let Some(commit) = args.get("commit") {
                parsed.as_of = Some(AsOfSelector::Commit(commit.clone()));
            } else if let Some(reference) = args.get("ref") {
                parsed.as_of = Some(AsOfSelector::Ref(reference.clone()));
            } else if let Some(save) = args.get("save") {
                if save.eq_ignore_ascii_case("current") {
                    parsed.as_of = Some(AsOfSelector::SaveCurrent);
                } else {
                    bail!("asOf(save:...) only supports save:\"current\"")
                }
            } else if let Some(revision) = args.get("saveRevision") {
                parsed.as_of = Some(AsOfSelector::SaveRevision(revision.clone()));
            } else {
                bail!("asOf(...) requires `commit:`, `ref:`, `save:`, or `saveRevision:`")
            }
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("file(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.file = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("files(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.files_path = args.get("path").cloned();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("artefacts(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_artefacts_stage = true;
            parsed.artefacts.kind = args.get("kind").cloned();
            parsed.artefacts.symbol_fqn = args.get("symbol_fqn").cloned();
            parsed.artefacts.agent = args.get("agent").cloned();
            parsed.artefacts.since = args.get("since").cloned();
            if let Some(lines) = args.get("lines") {
                parsed.artefacts.lines = Some(parse_lines_range(lines)?);
            }
            continue;
        }

        if stage == "artefacts()" {
            parsed.has_artefacts_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("deps(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_deps_stage = true;
            parsed.deps.kind = args.get("kind").cloned();
            if let Some(direction) = args.get("direction") {
                parsed.deps.direction = direction.clone();
            }
            if let Some(include_unresolved) = args.get("include_unresolved") {
                parsed.deps.include_unresolved = matches!(
                    include_unresolved.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes"
                );
            }
            continue;
        }

        if stage == "deps()" {
            parsed.has_deps_stage = true;
            continue;
        }

        if stage == "chatHistory()" {
            parsed.has_chat_history_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("checkpoints(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_checkpoints_stage = true;
            parsed.checkpoints.agent = args.get("agent").cloned();
            parsed.checkpoints.since = args.get("since").cloned();
            continue;
        }

        if stage == "checkpoints()" {
            parsed.has_checkpoints_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("telemetry(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_telemetry_stage = true;
            parsed.telemetry.event_type = args.get("event_type").cloned();
            parsed.telemetry.agent = args.get("agent").cloned();
            parsed.telemetry.since = args.get("since").cloned();
            continue;
        }

        if stage == "telemetry()" {
            parsed.has_telemetry_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("select(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.select_fields = inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("limit(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.limit = inner
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid limit value: {inner}"))?;
            continue;
        }

        bail!("unsupported DevQL stage: {stage}")
    }

    Ok(parsed)
}

fn validate_deps_filter(deps: &DepsFilter) -> Result<()> {
    const ALLOWED_KINDS: &[&str] = &[
        "imports",
        "calls",
        "references",
        "inherits",
        "implements",
        "exports",
    ];
    const ALLOWED_DIRECTIONS: &[&str] = &["out", "in", "both"];

    if let Some(kind) = deps.kind.as_deref() {
        let normalized = kind.to_ascii_lowercase();
        if !ALLOWED_KINDS.contains(&normalized.as_str()) {
            bail!(
                "deps(kind:...) must be one of: {}",
                ALLOWED_KINDS.join(", ")
            );
        }
    }

    let direction = deps.direction.to_ascii_lowercase();
    if !ALLOWED_DIRECTIONS.contains(&direction.as_str()) {
        bail!("deps(direction:...) must be one of: out, in, both");
    }

    Ok(())
}

fn parse_named_args(input: &str) -> Result<BTreeMap<String, String>> {
    let mut args = BTreeMap::new();
    if input.trim().is_empty() {
        return Ok(args);
    }

    let mut current = String::new();
    let mut pieces = Vec::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';

    for ch in input.chars() {
        match ch {
            '\'' | '"' => {
                if in_quotes && ch == quote_char {
                    in_quotes = false;
                    quote_char = '\0';
                } else if !in_quotes {
                    in_quotes = true;
                    quote_char = ch;
                }
                current.push(ch);
            }
            ',' if !in_quotes => {
                pieces.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }

    for piece in pieces {
        let Some((key, value)) = piece.split_once(':') else {
            bail!("invalid argument segment: {piece}")
        };
        let key = key.trim().to_string();
        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_single_quoted_or_double(value)?
        } else {
            value.to_string()
        };
        args.insert(key, value);
    }

    Ok(args)
}

fn parse_single_quoted_or_double(input: &str) -> Result<String> {
    let s = input.trim();
    if s.len() < 2 {
        bail!("expected quoted string, got: {input}")
    }

    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(s[1..s.len() - 1].to_string());
    }

    bail!("expected quoted string, got: {input}")
}

fn parse_lines_range(lines: &str) -> Result<(i32, i32)> {
    let Some((start, end)) = lines.split_once("..") else {
        bail!("invalid lines range: {lines}")
    };
    let start = start
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line start: {start}"))?;
    let end = end
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line end: {end}"))?;
    if start <= 0 || end <= 0 || end < start {
        bail!("invalid lines range: {lines}")
    }
    Ok((start, end))
}
