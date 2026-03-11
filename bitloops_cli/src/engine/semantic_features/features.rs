use super::common::{
    build_body_tokens, dedupe_tokens, normalize_name, normalize_string_list,
    split_identifier_tokens,
};
use super::{MAX_CONTEXT_TOKENS, MAX_IDENTIFIER_TOKENS, SemanticFeatureInput, normalize_repo_path};

#[derive(Debug, Clone, PartialEq)]
// Stores lexical and structural signals used later for matching and reranking.
// This is not the human-facing summary; it is the retrieval feature set.
pub struct SymbolFeaturesRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub prompt_version: String,
    pub normalized_name: String,
    pub normalized_signature: Option<String>,
    pub identifier_tokens: Vec<String>,
    pub normalized_body_tokens: Vec<String>,
    pub parent_kind: Option<String>,
    pub parent_symbol: Option<String>,
    pub parameter_count: Option<i32>,
    pub return_shape_hint: Option<String>,
    pub modifiers: Vec<String>,
    pub local_relationships: Vec<String>,
    pub context_tokens: Vec<String>,
}

pub(super) fn build_features_row(input: &SemanticFeatureInput) -> SymbolFeaturesRow {
    let normalized_signature = input.signature.as_deref().map(normalize_signature);
    let identifier_tokens = build_identifier_tokens(input);
    let normalized_body_tokens = build_body_tokens(&input.body);
    let modifiers = normalize_string_list(&input.modifiers);
    let local_relationships = normalize_string_list(&input.local_relationships);
    let context_tokens = build_context_tokens(input, &identifier_tokens);

    SymbolFeaturesRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        prompt_version: super::SYMBOL_FEATURES_PROMPT_VERSION.to_string(),
        normalized_name: normalize_name(&input.name),
        normalized_signature,
        identifier_tokens,
        normalized_body_tokens,
        parent_kind: input
            .parent_kind
            .clone()
            .map(|value| value.to_ascii_lowercase()),
        parent_symbol: input.parent_symbol.clone(),
        parameter_count: input.parameter_count,
        return_shape_hint: input.return_shape_hint.clone(),
        modifiers,
        local_relationships,
        context_tokens,
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

pub(super) fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn infer_return_shape_hint(
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

    let signature = signature.unwrap_or_default().to_ascii_lowercase();
    let combined = format!("{signature}\n{}", body.to_ascii_lowercase());

    if combined.contains("promise<") || combined.contains("-> promise") {
        return Some("promise".to_string());
    }
    if combined.contains("result<") || combined.contains("-> result") {
        return Some("result".to_string());
    }
    if combined.contains("option<") || combined.contains("-> option") {
        return Some("option".to_string());
    }
    if combined.contains("vec<")
        || combined.contains("[]")
        || combined.contains("array<")
        || combined.contains("list<")
    {
        return Some("collection".to_string());
    }
    if combined.contains("string") || combined.contains("to_string") {
        return Some("string".to_string());
    }
    if combined.contains("return true") || combined.contains("return false") {
        return Some("boolean".to_string());
    }

    None
}

pub(super) fn count_parameters_from_signature(signature: &str) -> Option<i32> {
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
