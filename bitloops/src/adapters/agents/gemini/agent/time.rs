use std::time::{SystemTime, UNIX_EPOCH};

use super::cli_agent::GeminiCliAgent;

impl GeminiCliAgent {
    pub(crate) fn current_utc_session_timestamp() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let (year, month, day, hour, minute, _) = Self::unix_to_ymdhms(secs);
        format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}")
    }

    pub(crate) fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
        let second = secs % 60;
        let minute = (secs / 60) % 60;
        let hour = (secs / 3600) % 24;

        let mut days = secs / 86_400;
        let mut year = 1970u64;
        loop {
            let year_days = if Self::is_leap(year) { 366 } else { 365 };
            if days < year_days {
                break;
            }
            days -= year_days;
            year += 1;
        }

        let month_lengths = [
            31u64,
            if Self::is_leap(year) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut month = 1u64;
        for len in month_lengths {
            if days < len {
                break;
            }
            days -= len;
            month += 1;
        }
        let day = days + 1;

        (year, month, day, hour, minute, second)
    }

    pub(crate) fn is_leap(year: u64) -> bool {
        (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
    }
}
