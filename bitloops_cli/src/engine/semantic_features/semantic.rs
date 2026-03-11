use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::common::split_identifier_tokens;
use super::{
    MAX_SUMMARY_BODY_CHARS, SEMANTIC_SUMMARY_PROMPT_VERSION, SemanticFeatureInput,
    normalize_repo_path,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSummaryCandidate {
    pub summary: String,
    pub confidence: f32,
    pub source_model: Option<String>,
}

pub trait SemanticSummaryProvider {
    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate>;

    fn prompt_version(&self) -> String {
        format!("{SEMANTIC_SUMMARY_PROMPT_VERSION}::provider=noop")
    }
}

pub struct NoopSemanticSummaryProvider;

impl SemanticSummaryProvider for NoopSemanticSummaryProvider {
    fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        None
    }
}

#[derive(Debug, serde::Deserialize)]
struct HostedSemanticSummaryJson {
    summary: String,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticSummaryProviderConfig {
    pub semantic_provider: Option<String>,
    pub semantic_model: Option<String>,
    pub semantic_api_key: Option<String>,
    pub semantic_base_url: Option<String>,
}

pub fn build_semantic_summary_provider(
    cfg: &SemanticSummaryProviderConfig,
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
        .ok_or_else(|| {
            anyhow!(
                "BITLOOPS_DEVQL_SEMANTIC_MODEL is required when semantic provider is configured"
            )
        })?
        .trim()
        .to_string();
    let api_key = cfg
        .semantic_api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "BITLOOPS_DEVQL_SEMANTIC_API_KEY is required when semantic provider is configured"
            )
        })?
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

pub fn resolve_semantic_summary_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(base_url.to_string());
    }

    match provider {
        "openai" => Ok("https://api.openai.com/v1/chat/completions".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions".to_string()),
        "openai_compatible" | "custom" => {
            bail!("BITLOOPS_DEVQL_SEMANTIC_BASE_URL is required for semantic provider `{provider}`")
        }
        other => bail!(
            "unsupported semantic provider `{other}`. Use `openai`, `openrouter`, or `openai_compatible` with BITLOOPS_DEVQL_SEMANTIC_BASE_URL"
        ),
    }
}

fn build_hosted_semantic_summary_prompt(input: &SemanticFeatureInput) -> String {
    let body = input.body.trim();
    let body = if body.chars().count() > MAX_SUMMARY_BODY_CHARS {
        body.chars()
            .take(MAX_SUMMARY_BODY_CHARS)
            .collect::<String>()
    } else {
        body.to_string()
    };

    format!(
        "Summarize this code symbol and return only JSON.\n\n\
JSON schema:\n\
{{\"summary\":\"One sentence about what the symbol does.\",\"confidence\":0.0}}\n\n\
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
                    "content": "You summarize code symbols. Return only JSON with keys summary and confidence."
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
pub enum SemanticSummarySource {
    DocComment,
    Llm,
    TemplateFallback,
}

impl SemanticSummarySource {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::DocComment => "doc_comment",
            Self::Llm => "llm",
            Self::TemplateFallback => "template_fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
// Stores all semantic summary candidates plus the currently preferred summary.
pub struct SymbolSemanticsRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub prompt_version: String,
    pub doc_comment_summary: Option<String>,
    pub llm_summary: Option<String>,
    pub template_summary: String,
    pub summary: String,
    pub confidence: f32,
    pub summary_source: SemanticSummarySource,
    pub source_model: Option<String>,
}
pub(super) fn build_semantics_row(
    input: &SemanticFeatureInput,
    summary_provider: &dyn SemanticSummaryProvider,
) -> SymbolSemanticsRow {
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
    let template_summary = build_template_summary(input);

    // Always persist all available summary candidates. The effective summary keeps a single
    // semantic string for downstream consumers such as embeddings and clone reranking.
    let (summary, confidence, summary_source) = if llm_summary_valid {
        (
            ensure_terminal_period(llm_summary.as_deref().unwrap_or_default()),
            llm_candidate
                .as_ref()
                .map(|candidate| candidate.confidence.clamp(0.0, 1.0))
                .unwrap_or(0.75_f32),
            SemanticSummarySource::Llm,
        )
    } else if let Some(doc_summary) = doc_comment_summary.as_ref() {
        (
            doc_summary.clone(),
            0.98_f32,
            SemanticSummarySource::DocComment,
        )
    } else {
        (
            template_summary.clone(),
            0.35_f32,
            SemanticSummarySource::TemplateFallback,
        )
    };

    SymbolSemanticsRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        prompt_version: summary_provider.prompt_version(),
        doc_comment_summary,
        llm_summary,
        template_summary,
        summary,
        confidence,
        summary_source,
        source_model: llm_candidate.and_then(|candidate| candidate.source_model),
    }
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

pub(super) fn normalize_summary_text(summary: &str) -> String {
    summary.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_template_summary(input: &SemanticFeatureInput) -> String {
    let subject = summary_subject(input);
    let summary = match input.canonical_kind.as_str() {
        "file" | "module" => format!("Defines the {} source file.", input.language),
        "class" | "interface" | "type" | "enum" | "variable" => format!("Defines {subject}."),
        "constructor" => format!("Constructs {subject}."),
        "test" => format!("Tests {subject}."),
        _ => format!("Implements {subject}."),
    };

    ensure_terminal_period(&summary)
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

fn summary_subject(input: &SemanticFeatureInput) -> String {
    if input.canonical_kind == "file" {
        return normalize_repo_path(&input.path).replace(['/', '.'], " ");
    }

    let tokens = split_identifier_tokens(&input.name);
    if tokens.is_empty() {
        input.name.to_ascii_lowercase()
    } else {
        tokens.join(" ")
    }
}
