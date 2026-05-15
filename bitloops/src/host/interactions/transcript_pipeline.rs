//! Host-level transcript entry pipeline.
//!
//! This module bridges between the per-agent `TranscriptEntryDeriver`
//! implementations and the dashboard resolvers. It provides:
//!
//! 1. **`derive_session_transcript_entries`** — derive canonical entries over
//!    the whole session, given the full transcript string. The resolver reads
//!    the live transcript file once and passes the content here.
//!
//! 2. **`derive_turn_transcript_entries`** — derive entries for a single turn
//!    by slicing the full transcript at the turn's
//!    `transcript_offset_start`/`transcript_offset_end` markers. The host
//!    lifecycle layer populates those markers at turn-end time, so the slice
//!    is *exactly* this turn's content — no cumulative duplication, no
//!    heuristic partitioning. Offset units are agent-specific (line numbers
//!    for JSONL, message indices for Gemini) and are handled by
//!    `Agent::slice_transcript_by_position`.
//!
//! 3. **`synthesize_prompt_fallback_entries`** — synthesize a single
//!    `USER/CHAT` entry from `turn.prompt` with `source = PROMPT_FALLBACK`
//!    for turns with no derivable transcript content (missing offsets,
//!    unreadable file, agent without a deriver, deriver returning empty).

use std::fs;

use crate::adapters::agents::Agent;
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_entry_id,
};
use crate::host::interactions::types::InteractionTurn;

/// Read the transcript file at `transcript_path` and return its contents.
///
/// Returns the empty string when the path is blank or the file is unreadable —
/// callers treat both as "no transcript available" and fall back to prompt
/// synthesis per-turn.
pub fn read_session_transcript_text(transcript_path: &str) -> String {
    if transcript_path.trim().is_empty() {
        return String::new();
    }
    match fs::read_to_string(transcript_path) {
        Ok(content) => content,
        Err(err) => {
            // Non-empty path but unreadable file (permissions, stale path
            // recorded in `interaction_sessions.transcript_path`, file pruned
            // by the agent). Log so operators can correlate empty-timeline
            // sessions with the underlying I/O failure; downstream still
            // degrades gracefully via prompt fallback.
            log::warn!("transcript read failed for path {transcript_path:?}: {err}");
            String::new()
        }
    }
}

/// Derive canonical transcript entries for the whole session from a
/// pre-read transcript string.
///
/// Returns an empty vec when the agent has no deriver, the transcript is
/// empty, or the deriver fails. Each entry has `turn_id = None` (session
/// scope); callers (e.g., the dashboard sidebar/tool-use tab) consume the
/// session-wide stream as-is.
pub fn derive_session_transcript_entries(
    session_id: &str,
    full_transcript: &str,
    agent: &dyn Agent,
) -> Vec<TranscriptEntry> {
    if full_transcript.is_empty() {
        return Vec::new();
    }
    let Some(deriver) = agent.as_transcript_entry_deriver() else {
        return Vec::new();
    };
    match deriver.derive_transcript_entries(session_id, None, full_transcript) {
        Ok(entries) => entries,
        Err(err) => {
            // Deriver bailed on the transcript — usually a malformed JSONL
            // line that serde rejected wholesale. Log so we don't silently
            // hand the dashboard an empty timeline. Callers still degrade
            // via prompt-fallback synthesis per turn.
            log::warn!(
                "transcript entry derivation failed for session {session_id} \
                 (agent={}): {err:#}",
                agent.agent_type()
            );
            Vec::new()
        }
    }
}

/// Derive entries for a single turn by slicing the full transcript at the
/// turn's offset markers.
///
/// Resolution order:
/// 1. If the agent has no deriver, fall back to prompt synthesis.
/// 2. If `transcript_offset_start`/`end` are missing or invalid (`end <= start`),
///    fall back to prompt synthesis.
/// 3. Ask the agent to slice the full transcript by position (JSONL line slice
///    for most agents, message-index slice for Gemini).
/// 4. If the slice is empty or the deriver returns no entries, fall back to
///    prompt synthesis.
/// 5. Otherwise return the derived entries with `turn_id = Some(turn.turn_id)`.
pub fn derive_turn_transcript_entries(
    session_id: &str,
    turn: &InteractionTurn,
    full_transcript: &str,
    agent: &dyn Agent,
) -> Vec<TranscriptEntry> {
    let Some(deriver) = agent.as_transcript_entry_deriver() else {
        return synthesize_prompt_fallback_entries(session_id, turn);
    };

    let start = turn.transcript_offset_start.unwrap_or(-1);
    let end = turn.transcript_offset_end.unwrap_or(-1);
    if start < 0 || end < 0 || end <= start || full_transcript.is_empty() {
        return synthesize_prompt_fallback_entries(session_id, turn);
    }

    let slice = agent.slice_transcript_by_position(full_transcript, start as usize, end as usize);
    if slice.is_empty() {
        return synthesize_prompt_fallback_entries(session_id, turn);
    }

    match deriver.derive_transcript_entries(session_id, Some(&turn.turn_id), &slice) {
        Ok(entries) if !entries.is_empty() => entries,
        Ok(_) => synthesize_prompt_fallback_entries(session_id, turn),
        Err(err) => {
            // Per-turn deriver failure (usually a malformed JSONL line in the
            // sliced range). Log so we can correlate a turn falling back to
            // PROMPT_FALLBACK with the upstream parse error.
            log::warn!(
                "per-turn transcript derivation failed for session {session_id} \
                 turn {turn_id} (agent={agent_type}, offsets={start}..{end}): {err:#}",
                turn_id = turn.turn_id,
                agent_type = agent.agent_type(),
            );
            synthesize_prompt_fallback_entries(session_id, turn)
        }
    }
}

/// Partition `session_entries` into one segment per turn using `USER/CHAT`
/// boundaries, then assign segments to turns sorted by turn_number.
///
/// This is the content-based fallback when offset-based slicing yields nothing
/// usable for one or more turns. Algorithm (mirrors the legacy frontend):
///
/// * Partition the session stream into segments. Each segment starts at a
///   `USER/CHAT` entry and continues until the next one (or end of stream).
/// * Pre-first-user noise (system messages, preamble) prepends to the first
///   segment so nothing is dropped.
/// * Sort turns by `turn_number`, then `started_at`, then `turn_id` for
///   determinism.
/// * Assign segment N to turn N. If `segments.len() > turns.len()`, append
///   overflow segments to the last turn. If `segments.len() < turns.len()`,
///   the extra turns get `PROMPT_FALLBACK` synthesis from `turn.prompt`.
/// * The returned `Vec` is indexed in the SAME order as `turns` (input order),
///   not turn_number order — caller-friendly for the dashboard resolver which
///   already has turns indexed by input position.
pub fn partition_session_entries_to_turns(
    session_id: &str,
    session_entries: &[TranscriptEntry],
    turns: &[&InteractionTurn],
) -> Vec<Vec<TranscriptEntry>> {
    let mut result: Vec<Vec<TranscriptEntry>> = vec![Vec::new(); turns.len()];
    if turns.is_empty() {
        return result;
    }

    // Sort turn indices by turn_number, then started_at, then turn_id.
    let mut sorted_indices: Vec<usize> = (0..turns.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        let ta = turns[a];
        let tb = turns[b];
        ta.turn_number
            .cmp(&tb.turn_number)
            .then_with(|| ta.started_at.cmp(&tb.started_at))
            .then_with(|| ta.turn_id.cmp(&tb.turn_id))
    });

    // Partition by USER/CHAT boundary.
    let mut segments: Vec<Vec<TranscriptEntry>> = Vec::new();
    let mut current: Vec<TranscriptEntry> = Vec::new();
    for entry in session_entries {
        let is_user_chat = matches!(entry.actor, TranscriptActor::User)
            && matches!(entry.variant, TranscriptVariant::Chat);
        if is_user_chat && !current.is_empty() {
            segments.push(std::mem::take(&mut current));
        }
        current.push(entry.clone());
    }
    if !current.is_empty() {
        segments.push(current);
    }

    // Pre-first-user noise (segments before the first USER/CHAT) prepends to
    // the first user-anchored segment to avoid dropping content.
    if segments.len() > 1 {
        let first_is_anchored = matches!(
            segments[0].first().map(|e| (e.actor, e.variant)),
            Some((TranscriptActor::User, TranscriptVariant::Chat))
        );
        if !first_is_anchored {
            let preamble = segments.remove(0);
            if let Some(first) = segments.first_mut() {
                let mut combined = preamble;
                combined.extend(std::mem::take(first));
                *first = combined;
            }
        }
    }

    // Assign segments to turns in sorted order.
    let segment_count = segments.len();
    for (seg_idx, segment) in segments.into_iter().enumerate() {
        if seg_idx >= sorted_indices.len() {
            // Overflow: extend last sorted turn
            if let Some(&last_turn_idx) = sorted_indices.last() {
                let last_turn_id = &turns[last_turn_idx].turn_id;
                result[last_turn_idx].extend(retag_entries(segment, last_turn_id));
            }
            continue;
        }
        let turn_idx = sorted_indices[seg_idx];
        let turn_id = &turns[turn_idx].turn_id;
        result[turn_idx] = retag_entries(segment, turn_id);
    }

    // Fill turns that got no segment with prompt fallback.
    if segment_count < sorted_indices.len() {
        for &turn_idx in sorted_indices.iter().skip(segment_count) {
            result[turn_idx] = synthesize_prompt_fallback_entries(session_id, turns[turn_idx]);
        }
    }

    result
}

fn retag_entries(entries: Vec<TranscriptEntry>, turn_id: &str) -> Vec<TranscriptEntry> {
    entries
        .into_iter()
        .map(|mut e| {
            e.turn_id = Some(turn_id.to_string());
            e
        })
        .collect()
}

/// Synthesize a single `USER/CHAT` entry from `turn.prompt` with
/// `source = PROMPT_FALLBACK`. Returns an empty vec when the prompt is blank.
///
/// Strips `<user_query>` wrapper tags before emitting, since Cursor (and
/// some other agents) wrap submitted prompts with these tags and store the
/// wrapped form in `turn.prompt`. The dashboard should show the bare text.
pub fn synthesize_prompt_fallback_entries(
    session_id: &str,
    turn: &InteractionTurn,
) -> Vec<TranscriptEntry> {
    let cleaned = strip_user_query_tags(&turn.prompt);
    let text = cleaned.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let scope = DerivationScope::Turn(&turn.turn_id);
    let timestamp = if turn.started_at.is_empty() {
        None
    } else {
        Some(turn.started_at.clone())
    };
    vec![TranscriptEntry {
        entry_id: make_entry_id(session_id, &scope, 0),
        session_id: session_id.to_string(),
        turn_id: Some(turn.turn_id.clone()),
        order: 0,
        timestamp,
        actor: TranscriptActor::User,
        variant: TranscriptVariant::Chat,
        source: TranscriptSource::PromptFallback,
        text: text.to_string(),
        tool_use_id: None,
        tool_kind: None,
        is_error: false,
    }]
}

/// Strip `<user_query>` / `</user_query>` wrapper tags (case-insensitive)
/// anywhere in `text`. Cursor and some other agents wrap submitted prompts
/// with these tags; the dashboard renders the plain prompt without them.
/// Returns the original string verbatim when no tag markers are present
/// (hot-path optimization for the common case).
pub fn strip_user_query_tags(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let lower = text.to_ascii_lowercase();
    if !lower.contains("<user_query>") && !lower.contains("</user_query>") {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let rest_lower = &lower[i..];
        if rest_lower.starts_with("<user_query>") {
            i += "<user_query>".len();
        } else if rest_lower.starts_with("</user_query>") {
            i += "</user_query>".len();
        } else {
            let ch = text[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::interactions::types::InteractionTurn;

    fn turn_with(prompt: &str, start: Option<i64>, end: Option<i64>) -> InteractionTurn {
        InteractionTurn {
            turn_id: "turn-1".to_string(),
            session_id: "sess-1".to_string(),
            repo_id: "repo-1".to_string(),
            turn_number: 1,
            prompt: prompt.to_string(),
            transcript_offset_start: start,
            transcript_offset_end: end,
            started_at: "2026-04-04T10:00:00Z".to_string(),
            ..InteractionTurn::default()
        }
    }

    #[test]
    fn synthesize_prompt_fallback_returns_user_chat_entry_when_prompt_non_empty() {
        let turn = turn_with("please run tests", None, None);
        let entries = synthesize_prompt_fallback_entries("sess-1", &turn);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, TranscriptActor::User);
        assert_eq!(entries[0].variant, TranscriptVariant::Chat);
        assert_eq!(entries[0].source, TranscriptSource::PromptFallback);
        assert_eq!(entries[0].text, "please run tests");
        assert_eq!(entries[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            entries[0].timestamp.as_deref(),
            Some("2026-04-04T10:00:00Z")
        );
    }

    #[test]
    fn synthesize_prompt_fallback_returns_empty_when_prompt_blank() {
        let turn = turn_with("   ", None, None);
        let entries = synthesize_prompt_fallback_entries("sess-1", &turn);
        assert!(entries.is_empty());
    }

    #[test]
    fn read_session_transcript_text_returns_empty_for_blank_path() {
        assert_eq!(read_session_transcript_text(""), "");
    }

    #[test]
    fn read_session_transcript_text_returns_empty_for_missing_file() {
        assert_eq!(
            read_session_transcript_text("/nonexistent/path/transcript.jsonl"),
            "",
        );
    }

    #[test]
    fn read_session_transcript_text_reads_file_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("transcript.jsonl");
        std::fs::write(&path, "hello world").expect("write");
        let content = read_session_transcript_text(path.to_string_lossy().as_ref());
        assert_eq!(content, "hello world");
    }

    #[test]
    fn derive_session_transcript_entries_returns_empty_for_empty_input() {
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_session_transcript_entries("sess-1", "", &agent);
        assert!(entries.is_empty());
    }

    #[test]
    fn derive_session_transcript_entries_runs_deriver_on_full_content() {
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let transcript = concat!(
            "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"first prompt\"}\n",
            "{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"first answer\",\"parts\":[]}\n",
        );
        let entries = derive_session_transcript_entries("sess-1", transcript, &agent);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "first prompt");
        assert_eq!(entries[1].text, "first answer");
        assert!(entries[0].turn_id.is_none()); // session scope
    }

    #[test]
    fn derive_turn_transcript_entries_slices_full_transcript_by_offsets() {
        // 3-turn cumulative transcript: each user/assistant pair takes 2 lines.
        let cumulative = concat!(
            "{\"id\":\"u1\",\"role\":\"user\",\"content\":\"first prompt\"}\n",
            "{\"id\":\"a1\",\"role\":\"assistant\",\"content\":\"first answer\",\"parts\":[]}\n",
            "{\"id\":\"u2\",\"role\":\"user\",\"content\":\"second prompt\"}\n",
            "{\"id\":\"a2\",\"role\":\"assistant\",\"content\":\"second answer\",\"parts\":[]}\n",
            "{\"id\":\"u3\",\"role\":\"user\",\"content\":\"third prompt\"}\n",
            "{\"id\":\"a3\",\"role\":\"assistant\",\"content\":\"third answer\",\"parts\":[]}\n",
        );
        // Turn 2 occupies lines 2..4 of the cumulative file.
        let turn = turn_with("second prompt", Some(2), Some(4));
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_turn_transcript_entries("sess-1", &turn, cumulative, &agent);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "second prompt");
        assert_eq!(entries[1].text, "second answer");
        assert_eq!(entries[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(entries[0].source, TranscriptSource::Transcript);
    }

    #[test]
    fn derive_turn_transcript_entries_falls_back_to_prompt_when_offsets_missing() {
        let cumulative = "{\"id\":\"u1\",\"role\":\"user\",\"content\":\"hello\"}\n";
        let turn = turn_with("the prompt", None, None);
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_turn_transcript_entries("sess-1", &turn, cumulative, &agent);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "the prompt");
        assert_eq!(entries[0].source, TranscriptSource::PromptFallback);
    }

    #[test]
    fn derive_turn_transcript_entries_falls_back_when_end_le_start() {
        let cumulative = "{\"id\":\"u1\",\"role\":\"user\",\"content\":\"hello\"}\n";
        let turn = turn_with("backup", Some(2), Some(2));
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_turn_transcript_entries("sess-1", &turn, cumulative, &agent);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, TranscriptSource::PromptFallback);
    }

    #[test]
    fn derive_turn_transcript_entries_falls_back_when_full_transcript_empty() {
        let turn = turn_with("backup", Some(0), Some(2));
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_turn_transcript_entries("sess-1", &turn, "", &agent);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source, TranscriptSource::PromptFallback);
    }

    #[test]
    fn derive_turn_transcript_entries_falls_back_when_deriver_returns_empty() {
        // Slice points at a line that doesn't match any role schema → deriver
        // returns nothing → fall through to prompt.
        let transcript = "{\"unrelated\":\"junk\"}\n";
        let turn = turn_with("backup prompt", Some(0), Some(1));
        let agent = crate::adapters::agents::open_code::agent_api::OpenCodeAgent;
        let entries = derive_turn_transcript_entries("sess-1", &turn, transcript, &agent);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "backup prompt");
        assert_eq!(entries[0].source, TranscriptSource::PromptFallback);
    }

    // ── partition_session_entries_to_turns ──────────────────────────────────
    //
    // The three branches under test:
    //   * USER/CHAT segmentation with pre-first-user preamble merging into
    //     the first segment so nothing is dropped.
    //   * Surplus segments (segments > turns) overflow onto the last turn.
    //   * Surplus turns (turns > segments) get `PROMPT_FALLBACK` synthesis
    //     from their `turn.prompt`.

    fn make_entry(
        actor: TranscriptActor,
        variant: TranscriptVariant,
        text: &str,
        order: i32,
    ) -> TranscriptEntry {
        TranscriptEntry {
            entry_id: format!("e-{order}"),
            session_id: "sess-1".to_string(),
            turn_id: None,
            order,
            timestamp: None,
            actor,
            variant,
            source: TranscriptSource::Transcript,
            text: text.to_string(),
            tool_use_id: None,
            tool_kind: None,
            is_error: false,
        }
    }

    fn make_turn(turn_id: &str, turn_number: u32, prompt: &str) -> InteractionTurn {
        InteractionTurn {
            turn_id: turn_id.to_string(),
            session_id: "sess-1".to_string(),
            repo_id: "repo-1".to_string(),
            turn_number,
            prompt: prompt.to_string(),
            started_at: format!("2026-04-04T10:0{turn_number}:00Z"),
            ..InteractionTurn::default()
        }
    }

    #[test]
    fn partition_segments_on_user_chat_boundaries_and_merges_preamble() {
        // Stream: SYSTEM preamble → USER/CHAT turn-1 → ASSISTANT/CHAT
        //         → USER/CHAT turn-2 → ASSISTANT/CHAT.
        // The SYSTEM preamble must NOT be dropped — it should prepend to the
        // first user-anchored segment so it ends up under turn-1.
        let entries = vec![
            make_entry(
                TranscriptActor::System,
                TranscriptVariant::Chat,
                "session prelude",
                0,
            ),
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 1", 1),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 1",
                2,
            ),
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 2", 3),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 2",
                4,
            ),
        ];
        let t1 = make_turn("turn-1", 1, "ask 1");
        let t2 = make_turn("turn-2", 2, "ask 2");
        let turns: Vec<&InteractionTurn> = vec![&t1, &t2];

        let partitioned = partition_session_entries_to_turns("sess-1", &entries, &turns);
        assert_eq!(partitioned.len(), 2);

        // turn-1 owns the preamble + its own USER/ASSISTANT pair.
        let texts_t1: Vec<&str> = partitioned[0].iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts_t1, vec!["session prelude", "ask 1", "answer 1"]);
        // All entries assigned to turn-1 carry its turn_id (retag pass).
        assert!(
            partitioned[0]
                .iter()
                .all(|e| e.turn_id.as_deref() == Some("turn-1"))
        );

        // turn-2 owns just its own USER/ASSISTANT pair.
        let texts_t2: Vec<&str> = partitioned[1].iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts_t2, vec!["ask 2", "answer 2"]);
        assert!(
            partitioned[1]
                .iter()
                .all(|e| e.turn_id.as_deref() == Some("turn-2"))
        );
    }

    #[test]
    fn partition_overflow_segments_append_to_last_turn() {
        // Three user-anchored segments but only two turns. The third segment
        // is overflow and must append onto the last turn (turn-2) rather than
        // being dropped.
        let entries = vec![
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 1", 0),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 1",
                1,
            ),
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 2", 2),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 2",
                3,
            ),
            // Third segment — no matching turn record exists.
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 3", 4),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 3",
                5,
            ),
        ];
        let t1 = make_turn("turn-1", 1, "ask 1");
        let t2 = make_turn("turn-2", 2, "ask 2");
        let turns: Vec<&InteractionTurn> = vec![&t1, &t2];

        let partitioned = partition_session_entries_to_turns("sess-1", &entries, &turns);
        assert_eq!(partitioned.len(), 2);

        let texts_t1: Vec<&str> = partitioned[0].iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts_t1, vec!["ask 1", "answer 1"]);

        // turn-2 gets its own segment *and* the overflow segment.
        let texts_t2: Vec<&str> = partitioned[1].iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts_t2, vec!["ask 2", "answer 2", "ask 3", "answer 3"]);
        // Overflow rows are retagged with the last turn's turn_id.
        assert!(
            partitioned[1]
                .iter()
                .all(|e| e.turn_id.as_deref() == Some("turn-2"))
        );
    }

    #[test]
    fn partition_surplus_turns_get_prompt_fallback_synthesis() {
        // One user-anchored segment but three turns. Turns 2 and 3 have no
        // corresponding segment and must be filled in with PROMPT_FALLBACK
        // entries synthesised from each turn's stored `prompt`.
        let entries = vec![
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 1", 0),
            make_entry(
                TranscriptActor::Assistant,
                TranscriptVariant::Chat,
                "answer 1",
                1,
            ),
        ];
        let t1 = make_turn("turn-1", 1, "ask 1");
        let t2 = make_turn("turn-2", 2, "fallback prompt 2");
        let t3 = make_turn("turn-3", 3, "fallback prompt 3");
        let turns: Vec<&InteractionTurn> = vec![&t1, &t2, &t3];

        let partitioned = partition_session_entries_to_turns("sess-1", &entries, &turns);
        assert_eq!(partitioned.len(), 3);

        // turn-1 gets the only real segment, source = Transcript.
        assert_eq!(partitioned[0].len(), 2);
        assert!(
            partitioned[0]
                .iter()
                .all(|e| e.source == TranscriptSource::Transcript)
        );

        // turn-2 and turn-3 get one PROMPT_FALLBACK USER/CHAT each, sourced
        // from the turn's stored prompt.
        for (idx, (turn_id, expected_text)) in [
            ("turn-2", "fallback prompt 2"),
            ("turn-3", "fallback prompt 3"),
        ]
        .iter()
        .enumerate()
        {
            let entries = &partitioned[idx + 1];
            assert_eq!(
                entries.len(),
                1,
                "{turn_id} should have 1 synthesised entry"
            );
            let entry = &entries[0];
            assert_eq!(entry.actor, TranscriptActor::User);
            assert_eq!(entry.variant, TranscriptVariant::Chat);
            assert_eq!(entry.source, TranscriptSource::PromptFallback);
            assert_eq!(entry.text, *expected_text);
            assert_eq!(entry.turn_id.as_deref(), Some(*turn_id));
        }
    }

    #[test]
    fn partition_preserves_input_order_even_when_turns_arrive_out_of_order() {
        // The function sorts turns by turn_number internally for segment
        // assignment but must return the result vec indexed in the SAME
        // order as the input slice. Verify by passing turns reversed.
        let entries = vec![
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 1", 0),
            make_entry(TranscriptActor::User, TranscriptVariant::Chat, "ask 2", 1),
        ];
        let t1 = make_turn("turn-1", 1, "ask 1");
        let t2 = make_turn("turn-2", 2, "ask 2");
        // Input order: t2 first, t1 second.
        let turns: Vec<&InteractionTurn> = vec![&t2, &t1];

        let partitioned = partition_session_entries_to_turns("sess-1", &entries, &turns);
        assert_eq!(partitioned.len(), 2);
        // Slot 0 is t2 (turn_number=2), so it gets segment 2 = "ask 2".
        assert_eq!(partitioned[0][0].text, "ask 2");
        assert_eq!(partitioned[0][0].turn_id.as_deref(), Some("turn-2"));
        // Slot 1 is t1 (turn_number=1), so it gets segment 1 = "ask 1".
        assert_eq!(partitioned[1][0].text, "ask 1");
        assert_eq!(partitioned[1][0].turn_id.as_deref(), Some("turn-1"));
    }
}
