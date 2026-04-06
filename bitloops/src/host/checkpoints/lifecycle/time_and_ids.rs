use uuid::Uuid;

pub(super) fn truncate_prompt_for_storage(prompt: &str) -> String {
    crate::utils::strings::truncate_runes(
        &prompt.split_whitespace().collect::<Vec<_>>().join(" "),
        100,
        "",
    )
}

pub(super) fn generate_lifecycle_turn_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

pub(super) fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;

    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if mo <= 2 { 1 } else { 0 };

    (year as u64, mo as u64, d as u64, h, mi, s)
}

pub(super) fn generate_interaction_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}
