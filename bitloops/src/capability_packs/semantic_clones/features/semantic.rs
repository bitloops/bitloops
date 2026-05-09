use crate::host::inference::{TextGenerationOptions, TextGenerationService};

use super::common::{normalize_repo_path, render_dependency_context, split_identifier_tokens};
use super::{MAX_SUMMARY_BODY_CHARS, SemanticFeatureInput};

const MINIMUM_SUMMARY_LENGTH: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSummaryCandidate {
    pub summary: String,
    pub confidence: Option<f32>,
    pub source_model: Option<String>,
}

pub trait SemanticSummaryProvider: Send + Sync {
    fn cache_key(&self) -> String;
    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate>;
    fn requires_model_output(&self) -> bool {
        false
    }
    fn persists_summaries(&self) -> bool {
        true
    }
    fn persists_summaries_for(&self, _input: &SemanticFeatureInput) -> bool {
        self.persists_summaries()
    }
}

pub fn summary_provider_from_service(
    service: std::sync::Arc<dyn TextGenerationService>,
    require_model_output: bool,
) -> std::sync::Arc<dyn SemanticSummaryProvider> {
    std::sync::Arc::new(TextGenerationServiceAdapter {
        service,
        require_model_output,
    })
}

pub struct NoopSemanticSummaryProvider;

impl SemanticSummaryProvider for NoopSemanticSummaryProvider {
    fn cache_key(&self) -> String {
        "provider=noop".to_string()
    }

    fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        None
    }

    fn persists_summaries(&self) -> bool {
        false
    }
}

pub struct DocstringOnlySummaryProvider;

impl SemanticSummaryProvider for DocstringOnlySummaryProvider {
    fn cache_key(&self) -> String {
        "provider=docstring_only".to_string()
    }

    fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        None
    }

    fn persists_summaries_for(&self, input: &SemanticFeatureInput) -> bool {
        extract_summary_from_docstring(input.docstring.as_deref()).is_some()
    }
}

pub struct DeterministicFallbackSummaryProvider;

impl SemanticSummaryProvider for DeterministicFallbackSummaryProvider {
    fn cache_key(&self) -> String {
        "provider=deterministic_fallback".to_string()
    }

    fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        None
    }
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

    let dependency_context = render_dependency_context(&input.dependency_signals);

    format!(
        "Summarise this code symbol in one plain sentence.\n\n\
Rules:\n\
- Return only the sentence text\n\
- Be specific to the symbol\n\
- Do not return JSON\n\
- Do not use markdown\n\n\
Context:\n\
language: {language}\n\
kind: {kind}\n\
language_kind: {language_kind}\n\
path: {path}\n\
symbol_fqn: {symbol_fqn}\n\
name: {name}\n\
signature: {signature}\n\
modifiers: {modifiers}\n\
docstring: {docstring}\n\
parent_kind: {parent_kind}\n\
dependencies: {dependencies}\n\
body:\n{body}",
        language = input.language,
        kind = input.canonical_kind,
        language_kind = input.language_kind,
        path = normalize_repo_path(&input.path),
        symbol_fqn = input.symbol_fqn,
        name = input.name,
        signature = input.signature.as_deref().unwrap_or(""),
        modifiers = input.modifiers.join(", "),
        docstring = input.docstring.as_deref().unwrap_or(""),
        parent_kind = input.parent_kind.as_deref().unwrap_or(""),
        dependencies = dependency_context,
        body = body,
    )
}

fn parse_semantic_summary_candidate_text(content: &str) -> Option<String> {
    let normalized = normalize_summary_text(content);
    let trimmed = normalized.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.contains("```") {
        return None;
    }
    if is_valid_summary(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

struct TextGenerationServiceAdapter {
    service: std::sync::Arc<dyn TextGenerationService>,
    require_model_output: bool,
}

impl SemanticSummaryProvider for TextGenerationServiceAdapter {
    fn cache_key(&self) -> String {
        format!("provider={}", self.service.cache_key())
    }

    fn generate(&self, input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
        let system_prompt =
            "You summarise code symbols. Return one plain sentence with no JSON and no markdown.";
        let user_prompt = build_semantic_summary_prompt(input);
        let content = match self.service.complete(system_prompt, &user_prompt) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    "semantic summary generation failed for `{}` (artefact `{}`): {err:#}",
                    input.path,
                    input.artefact_id
                );
                return None;
            }
        };
        if let Some(summary) = parse_semantic_summary_candidate_text(&content) {
            return Some(SemanticSummaryCandidate {
                summary,
                confidence: None,
                source_model: Some(self.service.descriptor()),
            });
        }

        log::warn!(
            "semantic summary provider returned an invalid plain-text response for `{}` (artefact `{}`)",
            input.path,
            input.artefact_id
        );

        let refreshed = match self.service.complete_with_options(
            system_prompt,
            &user_prompt,
            TextGenerationOptions {
                refresh_cache: true,
            },
        ) {
            Ok(content) => content,
            Err(err) => {
                log::warn!(
                    "semantic summary refresh failed for `{}` (artefact `{}`): {err:#}",
                    input.path,
                    input.artefact_id
                );
                return None;
            }
        };
        let Some(summary) = parse_semantic_summary_candidate_text(&refreshed) else {
            log::warn!(
                "semantic summary refresh returned an invalid plain-text response for `{}` (artefact `{}`)",
                input.path,
                input.artefact_id
            );
            return None;
        };
        Some(SemanticSummaryCandidate {
            summary,
            confidence: None,
            source_model: Some(self.service.descriptor()),
        })
    }

    fn requires_model_output(&self) -> bool {
        self.require_model_output
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
// Stores all semantic summary candidates plus the synthesized summary used downstream.
pub struct SymbolSemanticsRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub docstring_summary: Option<String>,
    pub llm_summary: Option<String>,
    pub template_summary: String,
    pub summary: String,
    pub confidence: Option<f32>,
    pub source_model: Option<String>,
}

impl SymbolSemanticsRow {
    pub fn is_llm_enriched(&self) -> bool {
        self.llm_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .source_model
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}
pub(super) fn build_semantics_row(
    input: &SemanticFeatureInput,
    summary_provider: &dyn SemanticSummaryProvider,
) -> SymbolSemanticsRow {
    let docstring_summary = extract_summary_from_docstring(input.docstring.as_deref());
    let llm_candidate = summary_provider.generate(input).and_then(|candidate| {
        let normalized_summary = normalize_summary_text(&candidate.summary);
        if !is_valid_summary(&normalized_summary) {
            return None;
        }
        Some(SemanticSummaryCandidate {
            summary: normalized_summary,
            confidence: candidate.confidence,
            source_model: candidate.source_model,
        })
    });
    let llm_summary = llm_candidate
        .as_ref()
        .map(|candidate| candidate.summary.clone());
    let canonical_llm_summary = llm_candidate
        .as_ref()
        .map(|candidate| ensure_terminal_period(candidate.summary.as_str()));
    let template_summary = build_template_summary(input);
    let llm_confidence = llm_candidate.as_ref().and_then(|candidate| {
        candidate
            .confidence
            .map(|confidence| confidence.clamp(0.0, 1.0))
    });
    let source_model = llm_candidate
        .as_ref()
        .and_then(|candidate| candidate.source_model.clone());

    // Persist every candidate, then synthesize a single canonical summary for Stage 3 and
    // other downstream consumers. Template stays as stable scaffolding, LLM adds the current
    // behavioral description when available, and docstrings remain a fallback/supporting hint.
    let summary = synthesize_summary(
        &template_summary,
        docstring_summary.as_deref(),
        canonical_llm_summary.as_deref(),
    );
    let confidence = synthesize_summary_confidence(
        docstring_summary.as_deref(),
        canonical_llm_summary.as_deref(),
        llm_confidence,
    );

    SymbolSemanticsRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        docstring_summary,
        llm_summary,
        template_summary,
        summary,
        confidence,
        source_model,
    }
}

fn extract_summary_from_docstring(docstring: Option<&str>) -> Option<String> {
    let normalized = normalize_summary_text(docstring?);
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

pub(crate) fn synthesize_deterministic_summary(
    template_summary: &str,
    docstring_summary: Option<&str>,
) -> String {
    synthesize_summary(template_summary, docstring_summary, None)
}

fn synthesize_summary(
    template_summary: &str,
    docstring_summary: Option<&str>,
    llm_summary: Option<&str>,
) -> String {
    let detail_summary = llm_summary.or(docstring_summary);
    let Some(detail_summary) = detail_summary else {
        return template_summary.to_string();
    };

    if summaries_equivalent(template_summary, detail_summary) {
        return ensure_terminal_period(detail_summary);
    }

    let template_sentence = ensure_terminal_period(template_summary);
    let detail_sentence = ensure_terminal_period(detail_summary);
    format!("{template_sentence} {detail_sentence}")
}

fn synthesize_summary_confidence(
    docstring_summary: Option<&str>,
    llm_summary: Option<&str>,
    _llm_confidence: Option<f32>,
) -> Option<f32> {
    match llm_summary {
        Some(_) => None,
        None if docstring_summary.is_some() => Some(0.68_f32),
        None => Some(0.35_f32),
    }
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
        && trimmed.len() >= MINIMUM_SUMMARY_LENGTH
        && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn summaries_equivalent(left: &str, right: &str) -> bool {
    summary_identity(left) == summary_identity(right)
}

fn summary_identity(summary: &str) -> String {
    normalize_summary_text(summary)
        .trim_end_matches(['.', '!', '?'])
        .to_ascii_lowercase()
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
    use std::sync::{Arc, Mutex};

    struct StaticSummaryProvider {
        summary: Option<String>,
        confidence: Option<f32>,
        source_model: Option<String>,
    }

    impl SemanticSummaryProvider for StaticSummaryProvider {
        fn cache_key(&self) -> String {
            "provider=test".to_string()
        }

        fn generate(&self, _input: &SemanticFeatureInput) -> Option<SemanticSummaryCandidate> {
            Some(SemanticSummaryCandidate {
                summary: self.summary.clone()?,
                confidence: self.confidence,
                source_model: self.source_model.clone(),
            })
        }
    }

    struct RecordingTextGenerationService {
        responses: Mutex<Vec<String>>,
        refresh_flags: Mutex<Vec<bool>>,
    }

    impl RecordingTextGenerationService {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(str::to_string).rev().collect()),
                refresh_flags: Mutex::new(Vec::new()),
            }
        }

        fn refresh_flags(&self) -> Vec<bool> {
            self.refresh_flags
                .lock()
                .expect("refresh flags lock")
                .clone()
        }
    }

    impl TextGenerationService for RecordingTextGenerationService {
        fn descriptor(&self) -> String {
            "bitloops:test-summary".to_string()
        }

        fn complete(&self, system_prompt: &str, user_prompt: &str) -> anyhow::Result<String> {
            self.complete_with_options(system_prompt, user_prompt, TextGenerationOptions::default())
        }

        fn complete_with_options(
            &self,
            _system_prompt: &str,
            _user_prompt: &str,
            options: TextGenerationOptions,
        ) -> anyhow::Result<String> {
            self.refresh_flags
                .lock()
                .expect("refresh flags lock")
                .push(options.refresh_cache);
            self.responses
                .lock()
                .expect("responses lock")
                .pop()
                .ok_or_else(|| anyhow::anyhow!("missing fake response"))
        }
    }

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
            modifiers: vec!["export".to_string()],
            body: "return value;".to_string(),
            docstring: None,
            parent_kind: Some("module".to_string()),
            dependency_signals: vec!["calls:user_repo::load_by_id".to_string()],
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn semantic_features_prompt_includes_context_and_truncates_body() {
        let mut input = sample_input("function", "normalizeEmail");
        input.docstring = Some("// Normalizes email.".to_string());
        input.body = "x".repeat(MAX_SUMMARY_BODY_CHARS + 50);

        let prompt = build_semantic_summary_prompt(&input);
        assert!(prompt.contains("docstring: // Normalizes email."));
        assert!(prompt.contains("modifiers: export"));
        assert!(prompt.contains("dependencies: calls:user repo::load by id"));
        assert!(prompt.contains("one plain sentence"));
        assert!(!prompt.contains("JSON schema"));
        assert!(!prompt.contains("confidence"));
        let body_section = prompt
            .split("body:\n")
            .nth(1)
            .expect("prompt should include body section");
        assert_eq!(body_section.chars().count(), MAX_SUMMARY_BODY_CHARS);
    }

    #[test]
    fn semantic_features_extract_summary_from_docstring_keeps_first_sentence() {
        let docstring = "Normalize email addresses before persistence. Keeps casing stable.";

        let summary = extract_summary_from_docstring(Some(docstring));
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
        let input = sample_input("method", "getById");

        assert_eq!(build_template_summary(&input), "Method get by id.");
    }

    #[test]
    fn semantic_features_synthesize_summary_combines_template_with_detail_sentence() {
        let summary = synthesize_summary(
            "Method get by id.",
            Some("Fetch a user record by its id."),
            Some("Loads a user entity by id from storage."),
        );

        assert_eq!(
            summary,
            "Method get by id. Loads a user entity by id from storage."
        );
    }

    #[test]
    fn semantic_features_synthesize_summary_uses_docstring_when_llm_missing() {
        let summary = synthesize_summary(
            "Function normalize email.",
            Some("Normalize email addresses before persistence."),
            None,
        );

        assert_eq!(
            summary,
            "Function normalize email. Normalize email addresses before persistence."
        );
    }

    #[test]
    fn semantic_features_synthesize_summary_avoids_duplicate_template_and_detail() {
        let summary = synthesize_summary(
            "Function normalize email.",
            Some("Function normalize email."),
            None,
        );

        assert_eq!(summary, "Function normalize email.");
    }

    #[test]
    fn semantic_features_build_semantics_row_drops_invalid_llm_candidate() {
        let input = sample_input("method", "getById");

        let row = build_semantics_row(
            &input,
            &StaticSummaryProvider {
                summary: Some("short".to_string()),
                confidence: Some(0.91),
                source_model: Some("ollama:ministral-3:3b".to_string()),
            },
        );

        assert_eq!(row.template_summary, "Method get by id.");
        assert_eq!(row.summary, "Method get by id.");
        assert_eq!(row.llm_summary, None);
        assert_eq!(row.source_model, None);
    }

    #[test]
    fn semantic_features_synthesize_summary_confidence_is_null_for_llm_summary() {
        let confidence = synthesize_summary_confidence(
            Some("Loads a user record by id from storage."),
            Some("Loads a user entity by id from storage."),
            Some(0.80),
        );

        assert_eq!(confidence, None);
    }

    #[test]
    fn semantic_features_synthesize_summary_confidence_keeps_deterministic_values_without_llm() {
        assert_eq!(
            synthesize_summary_confidence(Some("Loads a user by id."), None, None),
            Some(0.68)
        );
        assert_eq!(synthesize_summary_confidence(None, None, None), Some(0.35));
    }

    #[test]
    fn semantic_features_build_semantics_row_keeps_overlong_llm_summary_as_canonical_summary() {
        let input = sample_input("file", "cache");
        let llm_summary = "Creates and ensures the existence of a directory for caching local embeddings by validating and creating the specified path, defaulting to a user-specific cache location if no explicit path is provided.";
        assert!(
            llm_summary.chars().count() > 200,
            "test summary must stay over the previous hard limit"
        );

        let row = build_semantics_row(
            &input,
            &StaticSummaryProvider {
                summary: Some(llm_summary.to_string()),
                confidence: Some(0.91),
                source_model: Some("ollama:ministral-3:3b".to_string()),
            },
        );

        assert_eq!(row.llm_summary.as_deref(), Some(llm_summary));
        assert_eq!(
            row.summary,
            format!("Defines the typescript source file. {llm_summary}")
        );
        assert_eq!(row.confidence, None);
        assert_eq!(row.source_model.as_deref(), Some("ollama:ministral-3:3b"));
    }

    #[test]
    fn semantic_features_parse_semantic_summary_candidate_text_accepts_plain_sentence() {
        let parsed = parse_semantic_summary_candidate_text("Loads a user by id from storage.")
            .expect("plain summary should parse");

        assert_eq!(parsed, "Loads a user by id from storage.");
    }

    #[test]
    fn semantic_features_parse_semantic_summary_candidate_text_rejects_json_and_markdown() {
        assert_eq!(
            parse_semantic_summary_candidate_text(r#"{"summary":"Loads a user by id."}"#),
            None
        );
        assert_eq!(
            parse_semantic_summary_candidate_text("```json\n{\"summary\":\"Loads\"}\n```"),
            None
        );
    }

    #[test]
    fn semantic_features_text_generation_adapter_accepts_plain_summary_without_confidence() {
        let service = Arc::new(RecordingTextGenerationService::new(vec![
            "Loads a user entity by id from storage.",
        ]));
        let service_for_provider: Arc<dyn TextGenerationService> = service.clone();
        let provider = summary_provider_from_service(service_for_provider, false);

        let candidate = provider
            .generate(&sample_input("method", "getById"))
            .expect("plain summary");

        assert_eq!(candidate.summary, "Loads a user entity by id from storage.");
        assert_eq!(candidate.confidence, None);
        assert_eq!(service.refresh_flags(), vec![false]);
    }

    #[test]
    fn semantic_features_text_generation_adapter_refreshes_once_after_invalid_plain_summary() {
        let service = Arc::new(RecordingTextGenerationService::new(vec![
            r#"{"summary":"Cached JSON should be rejected."}"#,
            "Loads a user entity by id from storage.",
        ]));
        let service_for_provider: Arc<dyn TextGenerationService> = service.clone();
        let provider = summary_provider_from_service(service_for_provider, false);

        let candidate = provider
            .generate(&sample_input("method", "getById"))
            .expect("refreshed plain summary");

        assert_eq!(candidate.summary, "Loads a user entity by id from storage.");
        assert_eq!(candidate.confidence, None);
        assert_eq!(service.refresh_flags(), vec![false, true]);
    }
}
