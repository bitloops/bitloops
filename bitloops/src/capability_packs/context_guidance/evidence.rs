#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuidanceEvidenceInput {
    pub source: GuidanceEvidenceSource,
    pub title: Option<String>,
    pub body: Option<String>,
    pub prompt: Option<String>,
    pub transcript_fragment: Option<String>,
    pub target_paths: Vec<String>,
    pub target_symbols: Vec<String>,
    pub tool_events: Vec<GuidanceEvidenceToolEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuidanceEvidenceSource {
    History {
        checkpoint_id: Option<String>,
        session_id: String,
        turn_id: Option<String>,
        event_time: Option<String>,
        agent_type: Option<String>,
        model: Option<String>,
    },
    Knowledge {
        knowledge_item_id: String,
        knowledge_item_version_id: String,
        relation_assertion_id: Option<String>,
        provider: String,
        source_kind: String,
        title: Option<String>,
        url: Option<String>,
        updated_at: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuidanceEvidenceToolEvent {
    pub tool_kind: Option<String>,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub command: Option<String>,
}

const MAX_EVIDENCE_BODY_CHARS: usize = 6_000;
const MAX_EVIDENCE_TITLE_CHARS: usize = 300;

pub(super) fn bounded_evidence_text(value: Option<&str>) -> String {
    bounded_text(value.unwrap_or(""), MAX_EVIDENCE_BODY_CHARS)
}

pub(super) fn bounded_evidence_title(value: Option<&str>) -> String {
    bounded_text(value.unwrap_or(""), MAX_EVIDENCE_TITLE_CHARS)
}

pub(super) fn evidence_input_title(input: &GuidanceEvidenceInput) -> String {
    bounded_evidence_title(input.title.as_deref().or(match &input.source {
        GuidanceEvidenceSource::History { .. } => None,
        GuidanceEvidenceSource::Knowledge { title, .. } => title.as_deref(),
    }))
}

pub(super) fn evidence_input_body(input: &GuidanceEvidenceInput) -> String {
    bounded_evidence_text(input.body.as_deref())
}

pub(super) fn knowledge_source_label(input: &GuidanceEvidenceInput) -> Option<String> {
    match &input.source {
        GuidanceEvidenceSource::Knowledge {
            knowledge_item_id,
            knowledge_item_version_id,
            relation_assertion_id,
            provider,
            source_kind,
            title,
            url,
            updated_at,
        } => Some(
            [
                knowledge_item_id.as_str(),
                knowledge_item_version_id.as_str(),
                relation_assertion_id.as_deref().unwrap_or(""),
                provider.as_str(),
                source_kind.as_str(),
                title.as_deref().unwrap_or(""),
                url.as_deref().unwrap_or(""),
                updated_at.as_deref().unwrap_or(""),
            ]
            .join("\n"),
        ),
        GuidanceEvidenceSource::History { .. } => None,
    }
}

pub(super) fn evidence_target_symbols(input: &GuidanceEvidenceInput) -> &[String] {
    input.target_symbols.as_slice()
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let marker = "\n[... omitted for context guidance evidence budget ...]\n";
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars {
        return trimmed.chars().take(max_chars).collect();
    }
    let available = max_chars - marker_chars;
    let head_chars = available / 2;
    let tail_chars = available - head_chars;
    let head = trimmed.chars().take(head_chars).collect::<String>();
    let tail = trimmed
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{head}{marker}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_evidence_text_preserves_head_and_tail() {
        let value = format!("{}{}", "a".repeat(4_000), "z".repeat(4_000));

        let bounded = bounded_evidence_text(Some(value.as_str()));

        assert!(bounded.contains("omitted for context guidance evidence budget"));
        assert!(bounded.starts_with("aaa"));
        assert!(bounded.ends_with("zzz"));
        assert!(bounded.chars().count() <= MAX_EVIDENCE_BODY_CHARS);
    }

    #[test]
    fn knowledge_evidence_input_carries_source_label() {
        let input = GuidanceEvidenceInput {
            source: GuidanceEvidenceSource::Knowledge {
                knowledge_item_id: "item-1".to_string(),
                knowledge_item_version_id: "version-1".to_string(),
                relation_assertion_id: None,
                provider: "confluence".to_string(),
                source_kind: "page".to_string(),
                title: None,
                url: None,
                updated_at: None,
            },
            title: None,
            body: None,
            prompt: None,
            transcript_fragment: None,
            target_paths: Vec::new(),
            target_symbols: Vec::new(),
            tool_events: Vec::new(),
        };

        let label = knowledge_source_label(&input).expect("knowledge label");

        assert!(label.contains("item-1"));
        assert!(label.contains("version-1"));
        assert!(label.contains("confluence"));
        assert!(label.contains("page"));
    }
}
