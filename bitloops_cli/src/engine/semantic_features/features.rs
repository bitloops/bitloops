use super::common::{
    build_body_tokens, dedupe_tokens, normalize_name, normalize_repo_path, split_identifier_tokens,
};
use super::{MAX_CONTEXT_TOKENS, MAX_IDENTIFIER_TOKENS, SemanticFeatureInput};

#[derive(Debug, Clone, PartialEq)]
// Stores lexical and structural signals used later for matching and reranking.
// This is not the human-facing summary; it is the retrieval feature set.
pub struct SymbolFeaturesRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub normalized_name: String,
    pub normalized_signature: Option<String>,
    pub identifier_tokens: Vec<String>,
    pub normalized_body_tokens: Vec<String>,
    pub parent_kind: Option<String>,
    pub context_tokens: Vec<String>,
}

pub(super) fn build_features_row(input: &SemanticFeatureInput) -> SymbolFeaturesRow {
    let normalized_signature = input.signature.as_deref().map(normalize_signature);
    let identifier_tokens = build_identifier_tokens(input);
    let normalized_body_tokens = build_body_tokens(&input.body);
    let context_tokens = build_context_tokens(input, &identifier_tokens);

    SymbolFeaturesRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        normalized_name: normalize_name(&input.name),
        normalized_signature,
        identifier_tokens,
        normalized_body_tokens,
        parent_kind: input
            .parent_kind
            .clone()
            .map(|value| value.to_ascii_lowercase()),
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
    dedupe_tokens(tokens, MAX_IDENTIFIER_TOKENS)
}

fn build_context_tokens(input: &SemanticFeatureInput, identifier_tokens: &[String]) -> Vec<String> {
    let mut tokens = Vec::new();
    tokens.extend(split_identifier_tokens(&normalize_repo_path(&input.path)));
    if let Some(parent_kind) = &input.parent_kind {
        tokens.extend(split_identifier_tokens(parent_kind));
    }
    tokens.extend(identifier_tokens.iter().cloned());
    dedupe_tokens(tokens, MAX_CONTEXT_TOKENS)
}

pub(super) fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> SemanticFeatureInput {
        SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("symbol-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/services/user.ts".to_string(),
            language: "typescript".to_string(),
            canonical_kind: "method".to_string(),
            language_kind: "method".to_string(),
            symbol_fqn: "src/services/user.ts::UserService::getById".to_string(),
            name: "getById".to_string(),
            signature: Some(
                "async getById(id: string, opts: Map<string, Vec<i32>>): Promise<User>".to_string(),
            ),
            body: "return db.users.findById(id);".to_string(),
            docstring: None,
            parent_kind: Some("class".to_string()),
            content_hash: Some("hash-1".to_string()),
        }
    }

    #[test]
    fn semantic_features_normalize_signature_collapses_whitespace() {
        let normalized = normalize_signature("async   getById( id: string )  : Promise<User>");
        assert_eq!(normalized, "async getById( id: string ) : Promise<User>");
    }

    #[test]
    fn semantic_features_build_features_row_collects_identifier_and_context_tokens() {
        let row = build_features_row(&sample_input());

        assert_eq!(row.normalized_name, "get_by_id");
        assert!(
            row.identifier_tokens.contains(&"get".to_string())
                && row.identifier_tokens.contains(&"user".to_string())
        );
        assert!(
            row.context_tokens.contains(&"services".to_string())
                && row.context_tokens.contains(&"class".to_string())
        );
        assert!(
            row.normalized_body_tokens.contains(&"find".to_string()),
            "{:?}",
            row.normalized_body_tokens
        );
    }
}
