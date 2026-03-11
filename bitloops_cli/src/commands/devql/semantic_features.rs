const SYMBOL_FEATURES_PROMPT_VERSION: &str = "symbol-features-v2";
const SEMANTIC_SUMMARY_PROMPT_VERSION: &str = "semantic-summary-v1";
const MAX_IDENTIFIER_TOKENS: usize = 64;
const MAX_BODY_TOKENS: usize = 256;
const MAX_CONTEXT_TOKENS: usize = 64;
const MAX_SUMMARY_BODY_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct PreStageArtefactRow {
    artefact_id: String,
    #[serde(default)]
    symbol_id: Option<String>,
    repo_id: String,
    blob_sha: String,
    path: String,
    language: String,
    canonical_kind: String,
    language_kind: String,
    symbol_fqn: String,
    #[serde(default)]
    parent_artefact_id: Option<String>,
    #[serde(default)]
    start_line: Option<i32>,
    #[serde(default)]
    end_line: Option<i32>,
    #[serde(default)]
    start_byte: Option<i32>,
    #[serde(default)]
    end_byte: Option<i32>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct SemanticFeatureInput {
    artefact_id: String,
    symbol_id: Option<String>,
    repo_id: String,
    blob_sha: String,
    path: String,
    language: String,
    canonical_kind: String,
    language_kind: String,
    symbol_fqn: String,
    name: String,
    signature: Option<String>,
    body: String,
    doc_comment: Option<String>,
    parent_kind: Option<String>,
    parent_symbol: Option<String>,
    parameter_count: Option<i32>,
    return_shape_hint: Option<String>,
    modifiers: Vec<String>,
    local_relationships: Vec<String>,
    context_hints: Vec<String>,
    content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct SemanticSummaryCandidate {
    summary: String,
    role_tag: Option<String>,
    confidence: f32,
    source_model: Option<String>,
}

trait SemanticSummaryProvider {
    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate>;

    fn prompt_version(&self) -> String {
        format!("{SEMANTIC_SUMMARY_PROMPT_VERSION}::provider=noop")
    }
}

struct NoopSemanticSummaryProvider;

impl SemanticSummaryProvider for NoopSemanticSummaryProvider {
    fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        None
    }
}

#[derive(Debug, serde::Deserialize)]
struct HostedSemanticSummaryJson {
    summary: String,
    #[serde(default)]
    role_tag: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

fn build_semantic_summary_provider(
    cfg: &DevqlConfig,
) -> Result<Box<dyn SemanticSummaryProvider>> {
    let provider = cfg
        .semantic_provider
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if provider.is_empty() || provider == "none" || provider == "disabled" {
        return Ok(Box::new(NoopSemanticSummaryProvider));
    }

    let model = cfg
        .semantic_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("BITLOOPS_DEVQL_SEMANTIC_MODEL is required when semantic provider is configured"))?
        .trim()
        .to_string();
    let api_key = cfg
        .semantic_api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("BITLOOPS_DEVQL_SEMANTIC_API_KEY is required when semantic provider is configured"))?
        .trim()
        .to_string();
    let endpoint = resolve_semantic_summary_endpoint(&provider, cfg.semantic_base_url.as_deref())?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .context("building semantic summary HTTP client")?;

    Ok(Box::new(HostedSemanticSummaryProvider {
        provider,
        model,
        endpoint,
        api_key,
        client,
    }))
}

fn resolve_semantic_summary_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(base_url.to_string());
    }

    match provider {
        "openai" => Ok("https://api.openai.com/v1/chat/completions".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions".to_string()),
        "openai_compatible" | "custom" => bail!(
            "BITLOOPS_DEVQL_SEMANTIC_BASE_URL is required for semantic provider `{provider}`"
        ),
        other => bail!(
            "unsupported semantic provider `{other}`. Use `openai`, `openrouter`, or `openai_compatible` with BITLOOPS_DEVQL_SEMANTIC_BASE_URL"
        ),
    }
}

fn build_hosted_semantic_summary_prompt(input: &SemanticFeatureInput) -> String {
    let body = input.body.trim();
    let body = if body.chars().count() > MAX_SUMMARY_BODY_CHARS {
        body.chars().take(MAX_SUMMARY_BODY_CHARS).collect::<String>()
    } else {
        body.to_string()
    };

    format!(
        "Summarize this code symbol and return only JSON.\n\n\
JSON schema:\n\
{{\"summary\":\"One sentence about what the symbol does.\",\"role_tag\":\"short role label\",\"confidence\":0.0}}\n\n\
Rules:\n\
- summary must be a single sentence\n\
- summary must be specific to the symbol\n\
- confidence must be a float between 0 and 1\n\
- no markdown\n\n\
Context:\n\
language: {language}\n\
kind: {kind}\n\
language_kind: {language_kind}\n\
path: {path}\n\
symbol_fqn: {symbol_fqn}\n\
name: {name}\n\
signature: {signature}\n\
doc_comment: {doc_comment}\n\
parent_kind: {parent_kind}\n\
parent_symbol: {parent_symbol}\n\
parameter_count: {parameter_count}\n\
return_shape_hint: {return_shape_hint}\n\
modifiers: {modifiers}\n\
local_relationships: {local_relationships}\n\
context_hints: {context_hints}\n\
body:\n{body}",
        language = input.language,
        kind = input.canonical_kind,
        language_kind = input.language_kind,
        path = normalize_repo_path(&input.path),
        symbol_fqn = input.symbol_fqn,
        name = input.name,
        signature = input.signature.as_deref().unwrap_or(""),
        doc_comment = input.doc_comment.as_deref().unwrap_or(""),
        parent_kind = input.parent_kind.as_deref().unwrap_or(""),
        parent_symbol = input.parent_symbol.as_deref().unwrap_or(""),
        parameter_count = input
            .parameter_count
            .map(|value| value.to_string())
            .unwrap_or_default(),
        return_shape_hint = input.return_shape_hint.as_deref().unwrap_or(""),
        modifiers = input.modifiers.join(", "),
        local_relationships = input.local_relationships.join(", "),
        context_hints = input.context_hints.join(", "),
        body = body,
    )
}

fn extract_openai_compatible_message_content(value: &Value) -> Option<String> {
    let content = value.pointer("/choices/0/message/content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn parse_semantic_summary_candidate_json(content: &str) -> Option<HostedSemanticSummaryJson> {
    let payload = extract_json_object_from_text(content)?;
    serde_json::from_str::<HostedSemanticSummaryJson>(&payload).ok()
}

fn extract_json_object_from_text(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }

    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }

    Some(trimmed[start..=end].to_string())
}

struct HostedSemanticSummaryProvider {
    provider: String,
    model: String,
    endpoint: String,
    api_key: String,
    client: reqwest::blocking::Client,
}

impl SemanticSummaryProvider for HostedSemanticSummaryProvider {
    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        let payload = json!({
            "model": self.model,
            "temperature": 0.1,
            "messages": [
                {
                    "role": "system",
                    "content": "You summarize code symbols. Return only JSON with keys summary, role_tag, confidence."
                },
                {
                    "role": "user",
                    "content": build_hosted_semantic_summary_prompt(input),
                }
            ]
        });

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .ok()?;
        if !response.status().is_success() {
            log::warn!(
                "semantic summary provider request failed: provider={}, model={}, status={}",
                self.provider,
                self.model,
                response.status()
            );
            return None;
        }

        let value: Value = response.json().ok()?;
        let content = extract_openai_compatible_message_content(&value)?;
        let parsed = parse_semantic_summary_candidate_json(&content)?;
        Some(SemanticSummaryCandidate {
            summary: parsed.summary,
            role_tag: parsed.role_tag,
            confidence: parsed.confidence.unwrap_or(0.75),
            source_model: Some(format!("{}:{}", self.provider, self.model)),
        })
    }

    fn prompt_version(&self) -> String {
        format!(
            "{SEMANTIC_SUMMARY_PROMPT_VERSION}::provider={}::model={}",
            self.provider, self.model
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemanticSummarySource {
    DocComment,
    Llm,
    TemplateFallback,
}

impl SemanticSummarySource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::DocComment => "doc_comment",
            Self::Llm => "llm",
            Self::TemplateFallback => "template_fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
// Stores all semantic summary candidates plus the currently preferred summary.
struct SymbolSemanticsRow {
    artefact_id: String,
    repo_id: String,
    blob_sha: String,
    semantic_features_input_hash: String,
    prompt_version: String,
    doc_comment_summary: Option<String>,
    llm_summary: Option<String>,
    template_summary: String,
    summary: String,
    role_tag: String,
    confidence: f32,
    summary_source: SemanticSummarySource,
    source_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
// Stores lexical and structural signals used later for matching and reranking.
// This is not the human-facing summary; it is the retrieval feature set.
struct SymbolFeaturesRow {
    artefact_id: String,
    repo_id: String,
    blob_sha: String,
    semantic_features_input_hash: String,
    prompt_version: String,
    normalized_name: String,
    normalized_signature: Option<String>,
    identifier_tokens: Vec<String>,
    normalized_body_tokens: Vec<String>,
    parent_kind: Option<String>,
    parent_symbol: Option<String>,
    parameter_count: Option<i32>,
    return_shape_hint: Option<String>,
    modifiers: Vec<String>,
    local_relationships: Vec<String>,
    context_tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct SemanticFeatureRows {
    semantics: SymbolSemanticsRow,
    features: SymbolFeaturesRow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SemanticFeatureIndexState {
    semantics_hash: Option<String>,
    semantics_prompt_version: Option<String>,
    features_hash: Option<String>,
    features_prompt_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SemanticFeatureIngestionStats {
    upserted: usize,
    skipped: usize,
}

async fn load_pre_stage_artefacts_for_blob(
    pg_client: &tokio_postgres::Client,
    repo_id: &str,
    blob_sha: &str,
    path: &str,
) -> Result<Vec<PreStageArtefactRow>> {
    let sql = format!(
        "SELECT artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash \
FROM artefacts \
WHERE repo_id = '{repo_id}' AND blob_sha = '{blob_sha}' AND path = '{path}' \
ORDER BY coalesce(start_byte, 0), coalesce(start_line, 0), artefact_id",
        repo_id = esc_pg(repo_id),
        blob_sha = esc_pg(blob_sha),
        path = esc_pg(path),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(serde_json::from_value::<PreStageArtefactRow>(row)?);
    }
    Ok(artefacts)
}

fn build_semantic_feature_inputs_from_artefacts(
    artefacts: &[PreStageArtefactRow],
    blob_content: &str,
) -> Vec<SemanticFeatureInput> {
    let by_id = artefacts
        .iter()
        .map(|row| (row.artefact_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let child_kinds = build_child_kind_index(artefacts);

    artefacts
        .iter()
        .filter(|row| is_semantic_feature_candidate_kind(&row.canonical_kind))
        .map(|row| {
            build_semantic_feature_input_from_artefact(row, blob_content, &by_id, &child_kinds)
        })
        .collect()
}

fn build_child_kind_index(artefacts: &[PreStageArtefactRow]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for row in artefacts {
        let Some(parent_id) = row.parent_artefact_id.as_ref() else {
            continue;
        };
        out.entry(parent_id.clone())
            .or_default()
            .push(row.canonical_kind.clone());
    }
    out
}

fn build_semantic_feature_input_from_artefact(
    row: &PreStageArtefactRow,
    blob_content: &str,
    by_id: &HashMap<String, &PreStageArtefactRow>,
    child_kinds: &HashMap<String, Vec<String>>,
) -> SemanticFeatureInput {
    let parent = row
        .parent_artefact_id
        .as_ref()
        .and_then(|parent_id| by_id.get(parent_id))
        .copied();
    let body = extract_symbol_body(row, blob_content);
    let doc_comment = if row.canonical_kind == "file" {
        extract_file_header_comment(blob_content)
    } else {
        extract_symbol_doc_comment(blob_content, row.start_line)
    };
    let name = derive_symbol_name(row);
    let local_relationships = child_kinds
        .get(&row.artefact_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|kind| format!("contains:{kind}"))
        .collect::<Vec<_>>();
    let return_shape_hint = infer_return_shape_hint(row.signature.as_deref(), &body, None);
    let mut context_hints = vec![normalize_repo_path(&row.path)];
    context_hints.extend(
        parent
            .into_iter()
            .map(|parent_row| parent_row.symbol_fqn.clone())
            .collect::<Vec<_>>(),
    );

    SemanticFeatureInput {
        artefact_id: row.artefact_id.clone(),
        symbol_id: row.symbol_id.clone(),
        repo_id: row.repo_id.clone(),
        blob_sha: row.blob_sha.clone(),
        path: row.path.clone(),
        language: row.language.clone(),
        canonical_kind: row.canonical_kind.clone(),
        language_kind: row.language_kind.clone(),
        symbol_fqn: row.symbol_fqn.clone(),
        name,
        signature: row.signature.clone(),
        body,
        doc_comment,
        parent_kind: parent.map(|parent_row| parent_row.canonical_kind.clone()),
        parent_symbol: parent.map(|parent_row| parent_row.symbol_fqn.clone()),
        parameter_count: row
            .signature
            .as_deref()
            .and_then(count_parameters_from_signature),
        return_shape_hint,
        modifiers: extract_modifiers_from_signature(row.signature.as_deref()),
        local_relationships,
        context_hints,
        content_hash: row.content_hash.clone(),
    }
}

fn is_semantic_feature_candidate_kind(kind: &str) -> bool {
    matches!(
        kind,
        "file"
            | "module"
            | "function"
            | "method"
            | "class"
            | "interface"
            | "type"
            | "enum"
            | "variable"
            | "constructor"
            | "test"
    )
}

fn derive_symbol_name(row: &PreStageArtefactRow) -> String {
    if row.canonical_kind == "file" {
        return Path::new(&row.path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&row.path)
            .to_string();
    }

    row.symbol_fqn
        .rsplit("::")
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(&row.symbol_fqn)
        .to_string()
}

fn extract_symbol_body(row: &PreStageArtefactRow, blob_content: &str) -> String {
    if row.canonical_kind == "file" {
        return blob_content.to_string();
    }

    if let (Some(start_byte), Some(end_byte)) = (row.start_byte, row.end_byte) {
        let start_byte = start_byte.max(0) as usize;
        let end_byte = end_byte.max(start_byte as i32) as usize;
        if let Some(slice) = blob_content.get(start_byte..end_byte.min(blob_content.len())) {
            if !slice.trim().is_empty() {
                return slice.to_string();
            }
        }
    }

    if let (Some(start_line), Some(end_line)) = (row.start_line, row.end_line) {
        let lines = blob_content.lines().collect::<Vec<_>>();
        let start = start_line.max(1) as usize - 1;
        let end = end_line.max(start_line) as usize;
        if start < lines.len() {
            return lines[start..end.min(lines.len())].join("\n");
        }
    }

    row.signature.clone().unwrap_or_default()
}

fn extract_symbol_doc_comment(blob_content: &str, start_line: Option<i32>) -> Option<String> {
    let Some(start_line) = start_line else {
        return None;
    };
    if start_line <= 1 {
        return None;
    }

    let lines = blob_content.lines().collect::<Vec<_>>();
    let start_index = (start_line - 1) as usize;
    if start_index >= lines.len() {
        return None;
    }

    let mut comment_lines = Vec::new();
    let mut in_block = false;
    for index in (0..start_index).rev() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            break;
        }

        if in_block {
            comment_lines.push(trimmed.to_string());
            if trimmed.contains("/*") {
                break;
            }
            continue;
        }

        let is_comment = trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.ends_with("*/");
        if !is_comment {
            break;
        }

        comment_lines.push(trimmed.to_string());
        if trimmed.ends_with("*/") && !trimmed.contains("/*") {
            in_block = true;
        } else if trimmed.starts_with("/*") {
            break;
        }
    }

    if comment_lines.is_empty() {
        None
    } else {
        comment_lines.reverse();
        Some(comment_lines.join("\n"))
    }
}

fn extract_file_header_comment(content: &str) -> Option<String> {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let mut collected = Vec::new();
    let mut in_block = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if collected.is_empty() {
                continue;
            }
            break;
        }

        if in_block {
            collected.push(trimmed.to_string());
            if trimmed.contains("*/") {
                break;
            }
            continue;
        }

        if trimmed.starts_with("//") || trimmed.starts_with("/*") {
            collected.push(trimmed.to_string());
            if trimmed.starts_with("/*") && !trimmed.contains("*/") {
                in_block = true;
            } else if trimmed.starts_with("/*") {
                break;
            }
            continue;
        }

        break;
    }

    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n"))
    }
}

fn extract_modifiers_from_signature(signature: Option<&str>) -> Vec<String> {
    let Some(signature) = signature else {
        return Vec::new();
    };
    let lowered = signature.to_ascii_lowercase();
    let mut modifiers = Vec::new();
    for modifier in [
        "export",
        "default",
        "async",
        "pub",
        "static",
        "private",
        "protected",
        "readonly",
    ] {
        if lowered.contains(modifier) {
            modifiers.push(modifier.to_string());
        }
    }
    modifiers
}

async fn upsert_semantic_feature_rows(
    pg_client: &tokio_postgres::Client,
    inputs: &[SemanticFeatureInput],
    summary_provider: &dyn SemanticSummaryProvider,
) -> Result<SemanticFeatureIngestionStats> {
    let mut stats = SemanticFeatureIngestionStats::default();

    for input in inputs {
        let rows = build_semantic_feature_rows(input, summary_provider);
        let state = load_semantic_feature_index_state(pg_client, &input.artefact_id).await?;
        if !semantic_features_require_reindex(
            &state,
            &rows.semantics.semantic_features_input_hash,
            &rows.semantics.prompt_version,
            &rows.features.prompt_version,
        ) {
            stats.skipped += 1;
            continue;
        }

        persist_semantic_feature_rows(pg_client, &rows).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

fn build_semantic_feature_rows(
    input: &SemanticFeatureInput,
    summary_provider: &dyn SemanticSummaryProvider,
) -> SemanticFeatureRows {
    let semantic_features_input_hash = build_semantic_features_input_hash(input);
    let inferred_role_tag = infer_role_tag(input);
    let doc_comment_summary = extract_summary_from_doc_comment(input.doc_comment.as_deref());
    let llm_candidate = summary_provider.generate(input);
    let llm_summary = llm_candidate
        .as_ref()
        .map(|candidate| normalize_summary_text(&candidate.summary))
        .filter(|summary| !summary.is_empty());
    let llm_summary_valid = llm_summary
        .as_deref()
        .map(is_valid_summary)
        .unwrap_or(false);
    let template_summary = build_template_summary(input, &inferred_role_tag);

    // Always persist all available summary candidates. The effective summary keeps a single
    // semantic string for downstream consumers such as embeddings and clone reranking.
    let (summary, role_tag, confidence, summary_source) = if llm_summary_valid {
        (
            ensure_terminal_period(llm_summary.as_deref().unwrap_or_default()),
            llm_candidate
                .as_ref()
                .and_then(|candidate| candidate.role_tag.clone())
                .unwrap_or_else(|| inferred_role_tag.clone()),
            llm_candidate
                .as_ref()
                .map(|candidate| candidate.confidence.clamp(0.0, 1.0))
                .unwrap_or(0.75_f32),
            SemanticSummarySource::Llm,
        )
    } else if let Some(doc_summary) = doc_comment_summary.as_ref() {
        (
            doc_summary.clone(),
            inferred_role_tag.clone(),
            0.98_f32,
            SemanticSummarySource::DocComment,
        )
    } else {
        (
            template_summary.clone(),
            inferred_role_tag.clone(),
            0.35_f32,
            SemanticSummarySource::TemplateFallback,
        )
    };

    let normalized_signature = input.signature.as_deref().map(normalize_signature);
    let identifier_tokens = build_identifier_tokens(input);
    let normalized_body_tokens = build_body_tokens(&input.body);
    let modifiers = normalize_string_list(&input.modifiers);
    let local_relationships = normalize_string_list(&input.local_relationships);
    let context_tokens = build_context_tokens(input, &identifier_tokens);

    let semantics = SymbolSemanticsRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        semantic_features_input_hash: semantic_features_input_hash.clone(),
        prompt_version: summary_provider.prompt_version(),
        doc_comment_summary,
        llm_summary,
        template_summary,
        summary,
        role_tag,
        confidence,
        summary_source,
        source_model: llm_candidate.and_then(|candidate| candidate.source_model),
    };

    let features = SymbolFeaturesRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        semantic_features_input_hash,
        prompt_version: SYMBOL_FEATURES_PROMPT_VERSION.to_string(),
        normalized_name: normalize_name(&input.name),
        normalized_signature,
        identifier_tokens,
        normalized_body_tokens,
        parent_kind: input.parent_kind.clone().map(|value| value.to_ascii_lowercase()),
        parent_symbol: input.parent_symbol.clone(),
        parameter_count: input.parameter_count,
        return_shape_hint: input.return_shape_hint.clone(),
        modifiers,
        local_relationships,
        context_tokens,
    };

    SemanticFeatureRows {
        semantics,
        features,
    }
}

fn build_semantic_features_input_hash(input: &SemanticFeatureInput) -> String {
    sha256_hex(
        &json!({
            "artefact_id": &input.artefact_id,
            "symbol_id": &input.symbol_id,
            "repo_id": &input.repo_id,
            "blob_sha": &input.blob_sha,
            "path": normalize_repo_path(&input.path),
            "language": input.language.to_ascii_lowercase(),
            "canonical_kind": input.canonical_kind.to_ascii_lowercase(),
            "language_kind": input.language_kind.to_ascii_lowercase(),
            "symbol_fqn": &input.symbol_fqn,
            "name": normalize_name(&input.name),
            "signature": input.signature.as_deref().map(normalize_signature),
            "body_tokens": build_body_tokens(&input.body),
            "doc_comment": input
                .doc_comment
                .as_deref()
                .map(normalize_summary_text)
                .filter(|value| !value.is_empty()),
            "parent_kind": input.parent_kind.as_deref().map(|value| value.to_ascii_lowercase()),
            "parent_symbol": &input.parent_symbol,
            "parameter_count": input.parameter_count,
            "return_shape_hint": input
                .return_shape_hint
                .as_deref()
                .map(|value| value.to_ascii_lowercase()),
            "modifiers": normalize_string_list(&input.modifiers),
            "local_relationships": normalize_string_list(&input.local_relationships),
            "context_hints": normalize_string_list(&input.context_hints),
            "content_hash": &input.content_hash,
        })
        .to_string(),
    )
}

fn infer_role_tag(input: &SemanticFeatureInput) -> String {
    if input.path.contains("/tests/") || input.name.starts_with("test_") {
        return "test".to_string();
    }

    if input.canonical_kind == "class" && input.name.ends_with("Service") {
        return "service".to_string();
    }

    if input.canonical_kind == "variable" {
        return "constant".to_string();
    }

    let name_tokens = split_identifier_tokens(&input.name);
    let first = name_tokens.first().map(String::as_str).unwrap_or_default();
    match first {
        "parse" | "decode" | "tokenize" => "parser",
        "validate" | "assert" | "ensure" | "check" => "validator",
        "read" | "load" | "fetch" | "get" | "list" | "find" => "reader",
        "create" | "build" | "make" | "compose" => "builder",
        "update" | "set" | "write" | "save" | "persist" | "insert" | "append" => "writer",
        "handle" | "process" | "execute" | "run" | "dispatch" => "handler",
        "format" | "render" | "serialize" | "normalize" => "formatter",
        "query" | "select" | "filter" => "query_handler",
        _ => match input.canonical_kind.as_str() {
            "file" | "module" => "module",
            "class" | "interface" | "type" | "enum" => "type_definition",
            _ => "routine",
        },
    }
    .to_string()
}

fn build_template_summary(input: &SemanticFeatureInput, role_tag: &str) -> String {
    let subject = summary_subject(input);
    let summary = match role_tag {
        "parser" => format!("Parses {subject}."),
        "validator" => format!("Validates {subject}."),
        "reader" => format!("Loads {subject}."),
        "builder" => format!("Builds {subject}."),
        "writer" => format!("Updates {subject}."),
        "handler" => format!("Handles {subject}."),
        "formatter" => format!("Formats {subject}."),
        "query_handler" => format!("Queries {subject}."),
        "service" => format!("Coordinates {subject}."),
        "test" => format!("Tests {subject}."),
        "constant" => format!("Defines {subject}."),
        "module" => format!("Defines the {} source file.", input.language),
        "type_definition" => format!("Defines {subject}."),
        _ => format!("Implements {subject}."),
    };

    ensure_terminal_period(&summary)
}

fn summary_subject(input: &SemanticFeatureInput) -> String {
    if input.canonical_kind == "file" {
        return normalize_repo_path(&input.path)
            .replace('/', " ")
            .replace('.', " ");
    }

    let tokens = split_identifier_tokens(&input.name);
    let subject_tokens = match tokens.first().map(String::as_str) {
        Some(
            "parse"
                | "validate"
                | "assert"
                | "ensure"
                | "check"
                | "read"
                | "load"
                | "fetch"
                | "get"
                | "list"
                | "find"
                | "create"
                | "build"
                | "make"
                | "compose"
                | "update"
                | "set"
                | "write"
                | "save"
                | "persist"
                | "insert"
                | "append"
                | "handle"
                | "process"
                | "execute"
                | "run"
                | "dispatch"
                | "format"
                | "render"
                | "serialize"
                | "normalize"
                | "query"
                | "select"
                | "filter",
        ) if tokens.len() > 1 => &tokens[1..],
        _ => &tokens[..],
    };

    if subject_tokens.is_empty() {
        input.name.to_ascii_lowercase()
    } else {
        subject_tokens.join(" ")
    }
}

fn normalize_name(name: &str) -> String {
    let tokens = split_identifier_tokens(name);
    if tokens.is_empty() {
        name.trim().to_ascii_lowercase()
    } else {
        tokens.join("_")
    }
}

fn build_identifier_tokens(input: &SemanticFeatureInput) -> Vec<String> {
    let mut tokens = Vec::new();
    tokens.extend(split_identifier_tokens(&input.name));
    tokens.extend(split_identifier_tokens(&input.symbol_fqn));
    if let Some(signature) = &input.signature {
        tokens.extend(split_identifier_tokens(signature));
    }
    if let Some(parent) = &input.parent_symbol {
        tokens.extend(split_identifier_tokens(parent));
    }
    dedupe_tokens(tokens, MAX_IDENTIFIER_TOKENS)
}

fn build_body_tokens(body: &str) -> Vec<String> {
    dedupe_tokens(split_identifier_tokens(body), MAX_BODY_TOKENS)
}

fn build_context_tokens(input: &SemanticFeatureInput, identifier_tokens: &[String]) -> Vec<String> {
    let mut tokens = Vec::new();
    tokens.extend(split_identifier_tokens(&normalize_repo_path(&input.path)));
    if let Some(parent_kind) = &input.parent_kind {
        tokens.extend(split_identifier_tokens(parent_kind));
    }
    if let Some(parent_symbol) = &input.parent_symbol {
        tokens.extend(split_identifier_tokens(parent_symbol));
    }
    for hint in &input.context_hints {
        tokens.extend(split_identifier_tokens(hint));
    }
    for relationship in &input.local_relationships {
        tokens.extend(split_identifier_tokens(relationship));
    }
    tokens.extend(identifier_tokens.iter().cloned());
    dedupe_tokens(tokens, MAX_CONTEXT_TOKENS)
}

fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    dedupe_tokens(
        values
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
        MAX_CONTEXT_TOKENS,
    )
}

fn split_identifier_tokens(input: &str) -> Vec<String> {
    let regex = semantic_identifier_regex();
    let mut out = Vec::new();
    for capture in regex.find_iter(input) {
        out.extend(split_camel_case_word(capture.as_str()));
    }
    out.into_iter()
        .map(|token| token.to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn semantic_identifier_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"[A-Za-z0-9_]+").expect("identifier regex"))
}

fn split_camel_case_word(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars = input.chars().collect::<Vec<_>>();

    for (index, ch) in chars.iter().enumerate() {
        if *ch == '_' {
            if !current.is_empty() {
                tokens.push(current.to_ascii_lowercase());
                current.clear();
            }
            continue;
        }

        let should_split = if current.is_empty() {
            false
        } else {
            let prev = chars[index - 1];
            let next = chars.get(index + 1).copied();
            (ch.is_ascii_uppercase() && prev.is_ascii_lowercase())
                || (ch.is_ascii_digit() && prev.is_ascii_alphabetic())
                || (ch.is_ascii_alphabetic() && prev.is_ascii_digit())
                || (ch.is_ascii_uppercase()
                    && prev.is_ascii_uppercase()
                    && next.map(|value| value.is_ascii_lowercase()).unwrap_or(false))
        };

        if should_split && !current.is_empty() {
            tokens.push(current.to_ascii_lowercase());
            current.clear();
        }

        current.push(*ch);
    }

    if !current.is_empty() {
        tokens.push(current.to_ascii_lowercase());
    }

    tokens
}

fn dedupe_tokens(tokens: Vec<String>, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for token in tokens {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        out.push(normalized);
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn extract_summary_from_doc_comment(comment: Option<&str>) -> Option<String> {
    let cleaned = clean_comment_text(comment?);
    if cleaned.is_empty() {
        return None;
    }

    let first_sentence = cleaned
        .split_inclusive(['.', '!', '?'])
        .next()
        .unwrap_or(cleaned.as_str())
        .trim()
        .to_string();
    let normalized = normalize_summary_text(&first_sentence);
    if is_valid_summary(&normalized) {
        Some(ensure_terminal_period(&normalized))
    } else {
        None
    }
}

fn clean_comment_text(comment: &str) -> String {
    comment
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("///")
                .trim_start_matches("//!")
                .trim_start_matches("//")
                .trim_start_matches("/**")
                .trim_start_matches("/*")
                .trim_start_matches('*')
                .trim_end_matches("*/")
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty() && !line.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_summary_text(summary: &str) -> String {
    summary.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ensure_terminal_period(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if matches!(trimmed.chars().last(), Some('.') | Some('!') | Some('?')) {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

fn is_valid_summary(summary: &str) -> bool {
    let trimmed = summary.trim();
    !trimmed.is_empty()
        && !trimmed.contains('\n')
        && trimmed.len() >= 12
        && trimmed.len() <= 200
        && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn infer_return_shape_hint(
    signature: Option<&str>,
    body: &str,
    explicit_hint: Option<&str>,
) -> Option<String> {
    if let Some(hint) = explicit_hint {
        let normalized = hint.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }

    let combined = format!(
        "{} {}",
        signature.unwrap_or_default().to_ascii_lowercase(),
        body.to_ascii_lowercase()
    );

    for (pattern, hint) in [
        ("result<", "result"),
        ("option<", "option"),
        ("promise<", "promise"),
        ("bool", "boolean"),
        ("string", "string"),
        ("vec<", "collection"),
        ("[]", "collection"),
        ("iterator", "iterator"),
        ("void", "unit"),
        ("()", "unit"),
        ("null", "nullable"),
    ] {
        if combined.contains(pattern) {
            return Some(hint.to_string());
        }
    }

    if combined.contains("return true") || combined.contains("return false") {
        return Some("boolean".to_string());
    }

    None
}

fn count_parameters_from_signature(signature: &str) -> Option<i32> {
    let start = signature.find('(')?;
    let end = signature[start..].find(')')? + start;
    let inner = &signature[start + 1..end];
    if inner.trim().is_empty() {
        return Some(0);
    }

    let mut nesting = 0_i32;
    let mut count = 1_i32;
    for ch in inner.chars() {
        match ch {
            '<' | '(' | '[' | '{' => nesting += 1,
            '>' | ')' | ']' | '}' if nesting > 0 => nesting -= 1,
            ',' if nesting == 0 => count += 1,
            _ => {}
        }
    }

    Some(count)
}

async fn load_semantic_feature_index_state(
    pg_client: &tokio_postgres::Client,
    artefact_id: &str,
) -> Result<SemanticFeatureIndexState> {
    let sql = format!(
        "SELECT \
            (SELECT semantic_features_input_hash FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_hash, \
            (SELECT prompt_version FROM symbol_semantics WHERE artefact_id = '{artefact_id}') AS semantics_prompt_version, \
            (SELECT semantic_features_input_hash FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_hash, \
            (SELECT prompt_version FROM symbol_features WHERE artefact_id = '{artefact_id}') AS features_prompt_version",
        artefact_id = esc_pg(artefact_id),
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    let Some(row) = rows.first() else {
        return Ok(SemanticFeatureIndexState::default());
    };

    Ok(SemanticFeatureIndexState {
        semantics_hash: row
            .get("semantics_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        semantics_prompt_version: row
            .get("semantics_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_hash: row
            .get("features_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        features_prompt_version: row
            .get("features_prompt_version")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

// Incremental indexing rule: recompute enrichment only when symbol inputs or prompt versions change.
fn semantic_features_require_reindex(
    state: &SemanticFeatureIndexState,
    next_input_hash: &str,
    semantics_prompt_version: &str,
    features_prompt_version: &str,
) -> bool {
    state.semantics_hash.as_deref() != Some(next_input_hash)
        || state.features_hash.as_deref() != Some(next_input_hash)
        || state.semantics_prompt_version.as_deref() != Some(semantics_prompt_version)
        || state.features_prompt_version.as_deref() != Some(features_prompt_version)
}

async fn persist_semantic_feature_rows(
    pg_client: &tokio_postgres::Client,
    rows: &SemanticFeatureRows,
) -> Result<()> {
    let semantics = &rows.semantics;
    let features = &rows.features;

    let doc_comment_summary_expr = match semantics.doc_comment_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let llm_summary_expr = match semantics.llm_summary.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let source_model_expr = match semantics.source_model.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let normalized_signature_expr = match features.normalized_signature.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_kind_expr = match features.parent_kind.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parent_symbol_expr = match features.parent_symbol.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let parameter_count_expr = match features.parameter_count {
        Some(value) => value.to_string(),
        None => "NULL".to_string(),
    };
    let return_shape_expr = match features.return_shape_hint.as_deref() {
        Some(value) => format!("'{}'", esc_pg(value)),
        None => "NULL".to_string(),
    };
    let identifier_tokens =
        format!("'{}'::jsonb", esc_pg(&serde_json::to_string(&features.identifier_tokens)?));
    let body_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.normalized_body_tokens)?)
    );
    let modifiers = format!("'{}'::jsonb", esc_pg(&serde_json::to_string(&features.modifiers)?));
    let local_relationships = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.local_relationships)?)
    );
    let context_tokens = format!(
        "'{}'::jsonb",
        esc_pg(&serde_json::to_string(&features.context_tokens)?)
    );

    let sql = format!(
        "INSERT INTO symbol_semantics (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, doc_comment_summary, llm_summary, template_summary, summary, role_tag, confidence, summary_source, source_model) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{input_hash}', '{prompt_version}', {doc_comment_summary}, {llm_summary}, '{template_summary}', '{summary}', '{role_tag}', {confidence:.4}, '{summary_source}', {source_model}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, doc_comment_summary = EXCLUDED.doc_comment_summary, llm_summary = EXCLUDED.llm_summary, template_summary = EXCLUDED.template_summary, summary = EXCLUDED.summary, role_tag = EXCLUDED.role_tag, confidence = EXCLUDED.confidence, summary_source = EXCLUDED.summary_source, source_model = EXCLUDED.source_model, generated_at = now(); \
INSERT INTO symbol_features (artefact_id, repo_id, blob_sha, semantic_features_input_hash, prompt_version, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens, parent_kind, parent_symbol, parameter_count, return_shape_hint, modifiers, local_relationships, context_tokens) \
VALUES ('{features_artefact_id}', '{features_repo_id}', '{features_blob_sha}', '{features_input_hash}', '{features_prompt_version}', '{normalized_name}', {normalized_signature}, {identifier_tokens}, {body_tokens}, {parent_kind}, {parent_symbol}, {parameter_count}, {return_shape_hint}, {modifiers}, {local_relationships}, {context_tokens}) \
ON CONFLICT (artefact_id) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, semantic_features_input_hash = EXCLUDED.semantic_features_input_hash, prompt_version = EXCLUDED.prompt_version, normalized_name = EXCLUDED.normalized_name, normalized_signature = EXCLUDED.normalized_signature, identifier_tokens = EXCLUDED.identifier_tokens, normalized_body_tokens = EXCLUDED.normalized_body_tokens, parent_kind = EXCLUDED.parent_kind, parent_symbol = EXCLUDED.parent_symbol, parameter_count = EXCLUDED.parameter_count, return_shape_hint = EXCLUDED.return_shape_hint, modifiers = EXCLUDED.modifiers, local_relationships = EXCLUDED.local_relationships, context_tokens = EXCLUDED.context_tokens, generated_at = now()",
        artefact_id = esc_pg(&semantics.artefact_id),
        repo_id = esc_pg(&semantics.repo_id),
        blob_sha = esc_pg(&semantics.blob_sha),
        input_hash = esc_pg(&semantics.semantic_features_input_hash),
        prompt_version = esc_pg(&semantics.prompt_version),
        doc_comment_summary = doc_comment_summary_expr,
        llm_summary = llm_summary_expr,
        template_summary = esc_pg(&semantics.template_summary),
        summary = esc_pg(&semantics.summary),
        role_tag = esc_pg(&semantics.role_tag),
        confidence = semantics.confidence,
        summary_source = semantics.summary_source.as_str(),
        source_model = source_model_expr,
        features_artefact_id = esc_pg(&features.artefact_id),
        features_repo_id = esc_pg(&features.repo_id),
        features_blob_sha = esc_pg(&features.blob_sha),
        features_input_hash = esc_pg(&features.semantic_features_input_hash),
        features_prompt_version = esc_pg(&features.prompt_version),
        normalized_name = esc_pg(&features.normalized_name),
        normalized_signature = normalized_signature_expr,
        identifier_tokens = identifier_tokens,
        body_tokens = body_tokens,
        parent_kind = parent_kind_expr,
        parent_symbol = parent_symbol_expr,
        parameter_count = parameter_count_expr,
        return_shape_hint = return_shape_expr,
        modifiers = modifiers,
        local_relationships = local_relationships,
        context_tokens = context_tokens,
    );

    postgres_exec(pg_client, &sql).await
}

fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
