use anyhow::{Result, anyhow};

use crate::engine::providers::llm::{LlmProvider, build_llm_provider};

use super::common::{normalize_repo_path, split_identifier_tokens};
use super::{MAX_SUMMARY_BODY_CHARS, SEMANTIC_SUMMARY_PROMPT_VERSION, SemanticFeatureInput};

pub use crate::engine::providers::llm::resolve_semantic_summary_endpoint;

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
    Ok(Box::new(LlmSemanticSummaryProvider {
        llm_provider: build_llm_provider(
            &provider,
            model,
            api_key,
            cfg.semantic_base_url.as_deref(),
        )?,
    }))
}

fn build_semantic_summary_prompt(input: &SemanticFeatureInput) -> String {
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
        local_relationships = input.local_relationships.join(", "),
        context_hints = input.context_hints.join(", "),
        body = body,
    )
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

struct LlmSemanticSummaryProvider {
    llm_provider: Box<dyn LlmProvider>,
}

impl SemanticSummaryProvider for LlmSemanticSummaryProvider {
    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        let content = self.llm_provider.complete(
            "You summarize code symbols. Return only JSON with keys summary and confidence.",
            &build_semantic_summary_prompt(input),
        )?;
        let parsed = parse_semantic_summary_candidate_json(&content)?;
        Some(SemanticSummaryCandidate {
            summary: parsed.summary,
            confidence: parsed.confidence.unwrap_or(0.75),
            source_model: Some(self.llm_provider.descriptor()),
        })
    }

    fn prompt_version(&self) -> String {
        self.llm_provider
            .prompt_version(SEMANTIC_SUMMARY_PROMPT_VERSION)
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
    let normalized = normalize_summary_text(comment?);
    if normalized.is_empty() {
        return None;
    }

    let first_sentence = normalized
        .split_inclusive(['.', '!', '?'])
        .next()
        .unwrap_or(normalized.as_str())
        .trim()
        .to_string();
    if is_valid_summary(&first_sentence) {
        Some(ensure_terminal_period(&first_sentence))
    } else {
        None
    }
}

pub(super) fn normalize_summary_text(summary: &str) -> String {
    summary.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_template_summary(input: &SemanticFeatureInput) -> String {
    let summary = match input.canonical_kind.as_str() {
        "file" | "module" => format!("Defines the {} source file.", input.language),
        _ => format!(
            "{} {}.",
            canonical_kind_label(&input.canonical_kind),
            summary_subject(input)
        ),
    };

    ensure_terminal_period(&summary)
}

fn canonical_kind_label(kind: &str) -> String {
    let normalized = kind.trim().replace('_', " ");
    let mut chars = normalized.chars();
    let Some(first) = chars.next() else {
        return "Symbol".to_string();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(kind: &str, name: &str) -> SemanticFeatureInput {
        SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("symbol-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/services/user.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: kind.to_string(),
            language_kind: kind.to_string(),
            symbol_fqn: format!("src/services/user.ts::{name}"),
            name: name.to_string(),
            signature: Some(format!("function {name}()")),
            body: "return value;".to_string(),
            doc_comment: None,
            parent_kind: Some("module".to_string()),
            parent_symbol: Some("src/services/user.ts".to_string()),
            local_relationships: vec![],
            context_hints: vec!["src/services/user.ts".to_string()],
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn semantic_features_build_provider_supports_disabled_and_requires_api_key() {
        let disabled = build_semantic_summary_provider(&SemanticSummaryProviderConfig {
            semantic_provider: Some("disabled".to_string()),
            ..SemanticSummaryProviderConfig::default()
        })
        .expect("disabled provider should build");
        assert_eq!(
            disabled.prompt_version(),
            "semantic-summary-v4::provider=noop"
        );

        let err = build_semantic_summary_provider(&SemanticSummaryProviderConfig {
            semantic_provider: Some("openai".to_string()),
            semantic_model: Some("gpt-test".to_string()),
            ..SemanticSummaryProviderConfig::default()
        })
        .err()
        .expect("missing API key should fail");
        assert!(
            err.to_string()
                .contains("BITLOOPS_DEVQL_SEMANTIC_API_KEY is required")
        );
    }

    #[test]
    fn semantic_features_prompt_includes_context_and_truncates_body() {
        let mut input = sample_input("function", "normalizeEmail");
        input.doc_comment = Some("// Normalizes email.".to_string());
        input.local_relationships = vec!["contains:validation".to_string()];
        input.context_hints = vec!["src/services/user.ts".to_string()];
        input.body = "x".repeat(MAX_SUMMARY_BODY_CHARS + 50);

        let prompt = build_semantic_summary_prompt(&input);
        assert!(prompt.contains("doc_comment: // Normalizes email."));
        assert!(prompt.contains("local_relationships: contains:validation"));
        assert!(prompt.contains("context_hints: src/services/user.ts"));
        let body_section = prompt
            .split("body:\n")
            .nth(1)
            .expect("prompt should include body section");
        assert_eq!(body_section.chars().count(), MAX_SUMMARY_BODY_CHARS);
    }

    #[test]
    fn semantic_features_extract_summary_from_doc_comment_keeps_first_sentence() {
        let comment = "Normalize email addresses before persistence. Keeps casing stable.";

        let summary = extract_summary_from_doc_comment(Some(comment));
        assert_eq!(
            summary.as_deref(),
            Some("Normalize email addresses before persistence.")
        );
    }

    #[test]
    fn semantic_features_template_summary_uses_neutral_kind_label() {
        let input = sample_input("function", "normalizeEmail");
        assert_eq!(build_template_summary(&input), "Function normalize email.");
    }

    #[test]
    fn semantic_features_template_summary_keeps_file_special_case() {
        let mut input = sample_input("file", "user");
        input.path = "src/services/user.ts".to_string();
        assert_eq!(
            build_template_summary(&input),
            "Defines the typescript source file."
        );
    }

    #[test]
    fn semantic_features_template_summary_without_prestage_contract() {
        let mut input = sample_input("method", "getById");

        assert_eq!(build_template_summary(&input), "Method get by id.");
    }

    #[test]
    fn semantic_features_parse_semantic_summary_candidate_json_from_wrapped_text() {
        let parsed = parse_semantic_summary_candidate_json(
            r#"Here is the result: {"summary":"Loads a user by id.","confidence":0.82}"#,
        )
        .expect("wrapped JSON should parse");

        assert_eq!(parsed.summary, "Loads a user by id.");
        assert_eq!(parsed.confidence, Some(0.82));
    }

    #[test]
    fn semantic_features_extract_json_object_returns_none_for_invalid_wrappers() {
        assert_eq!(extract_json_object_from_text(""), None);
        assert_eq!(extract_json_object_from_text("no json here"), None);
        assert_eq!(extract_json_object_from_text("{missing"), None);
    }
}
