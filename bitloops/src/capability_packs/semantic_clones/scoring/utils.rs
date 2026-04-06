use super::*;

pub(super) fn summary_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> (f32, Vec<String>) {
    let source_tokens = summary_tokens(&source.summary);
    let target_tokens = summary_tokens(&target.summary);
    jaccard_with_shared(&source_tokens, &target_tokens)
}

pub(super) fn summary_tokens(summary: &str) -> Vec<String> {
    summary
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            if is_informative_signal_token(&token) {
                Some(token)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
}

pub(super) fn filter_signal_tokens(tokens: Vec<String>) -> Vec<String> {
    tokens
        .into_iter()
        .filter(|token| is_informative_signal_token(token))
        .take(SHARED_SIGNAL_EXPLANATION_LIMIT)
        .collect()
}

pub(super) fn is_informative_signal_token(token: &str) -> bool {
    token.len() >= 3 && token.chars().any(|ch| ch.is_ascii_alphabetic())
}

pub(super) fn container_identity(symbol_fqn: &str) -> Option<String> {
    let segments = symbol_fqn
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        return None;
    }

    Some(segments[..segments.len() - 1].join("::"))
}

pub(super) fn bool_score(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

pub(super) fn build_clone_input_hash(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> String {
    sha256_hex(
        &json!({
            "fingerprint_version": SYMBOL_CLONE_FINGERPRINT_VERSION,
            "source_symbol_id": &source.symbol_id,
            "target_symbol_id": &target.symbol_id,
            "source_artefact_id": &source.artefact_id,
            "target_artefact_id": &target.artefact_id,
            "source_summary": &source.summary,
            "target_summary": &target.summary,
            "source_name": &source.normalized_name,
            "target_name": &target.normalized_name,
            "source_signature": &source.normalized_signature,
            "target_signature": &target.normalized_signature,
            "source_body_tokens": &source.normalized_body_tokens,
            "target_body_tokens": &target.normalized_body_tokens,
            "source_calls": &source.call_targets,
            "target_calls": &target.call_targets,
            "source_dependencies": &source.dependency_targets,
            "target_dependencies": &target.dependency_targets,
            "source_embedding_provider": &source.embedding_provider,
            "target_embedding_provider": &target.embedding_provider,
            "source_embedding_model": &source.embedding_model,
            "target_embedding_model": &target.embedding_model,
            "source_embedding_dimension": source.embedding_dimension,
            "target_embedding_dimension": target.embedding_dimension,
            "source_churn": source.churn_count,
            "target_churn": target.churn_count,
        })
        .to_string(),
    )
}

pub(super) fn compatible_kind_score(left: &str, right: &str) -> f32 {
    if same_clone_kind(left, right) {
        return 1.0;
    }
    0.0
}

pub(super) fn same_clone_kind(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

pub(super) fn is_meaningful_clone_candidate(input: &SymbolCloneCandidateInput) -> bool {
    if input.canonical_kind.eq_ignore_ascii_case("import") {
        return false;
    }
    true
}

pub(super) fn signature_similarity(
    source: &SymbolCloneCandidateInput,
    target: &SymbolCloneCandidateInput,
) -> f32 {
    match (&source.normalized_signature, &target.normalized_signature) {
        (Some(left), Some(right)) if left == right => 1.0,
        (Some(_), Some(_)) => MISSING_SIGNATURE_SCORE,
        (None, None) => 1.0,
        _ => MISSING_SIGNATURE_SCORE,
    }
}

pub(super) fn name_match_score(left: &str, right: &str) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    if left.starts_with(right) || right.starts_with(left) {
        return PARTIAL_NAME_MATCH_SCORE;
    }

    let left_tokens = left.split('_').collect::<Vec<_>>();
    let right_tokens = right.split('_').collect::<Vec<_>>();
    let shared_prefix = left_tokens
        .iter()
        .zip(right_tokens.iter())
        .take_while(|(left, right)| left == right)
        .count();
    match shared_prefix {
        2.. => PARTIAL_NAME_MATCH_SCORE,
        1 => SINGLE_SHARED_NAME_PREFIX_SCORE,
        _ => 0.0,
    }
}

pub(super) fn path_similarity(left: &str, right: &str) -> f32 {
    let left_segments = left
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let right_segments = right
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if left_segments.is_empty() || right_segments.is_empty() {
        return 0.0;
    }

    let mut shared = 0usize;
    for (left, right) in left_segments.iter().zip(right_segments.iter()) {
        if left == right {
            shared += 1;
        } else {
            break;
        }
    }

    shared as f32 / left_segments.len().max(right_segments.len()) as f32
}

pub(super) fn jaccard_with_shared(left: &[String], right: &[String]) -> (f32, Vec<String>) {
    let left_set = left
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let right_set = right
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();

    if left_set.is_empty() && right_set.is_empty() {
        return (1.0, Vec::new());
    }
    if left_set.is_empty() || right_set.is_empty() {
        return (0.0, Vec::new());
    }

    let shared = left_set
        .intersection(&right_set)
        .cloned()
        .collect::<BTreeSet<_>>();
    let union_count = left_set.union(&right_set).count();
    (
        shared.len() as f32 / union_count as f32,
        shared
            .into_iter()
            .take(SHARED_SIGNAL_EXPLANATION_LIMIT)
            .collect(),
    )
}

pub(super) fn normalized_body_hash(input: &SymbolCloneCandidateInput) -> String {
    sha256_hex(&input.normalized_body_tokens.join("|"))
}

pub(super) fn normalized_signature_hash(input: &SymbolCloneCandidateInput) -> String {
    sha256_hex(
        &json!({
            "kind": input.canonical_kind,
            "parent_kind": input.parent_kind,
            "normalized_signature": input.normalized_signature,
        })
        .to_string(),
    )
}

pub(super) fn is_experimental_path(path: &str) -> bool {
    let normalized = path.to_ascii_lowercase();
    normalized.contains("/experimental/")
        || normalized.contains("/playground/")
        || normalized.contains("/tmp/")
        || normalized.contains("/fixtures/")
}

pub(super) fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
