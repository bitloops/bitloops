//! Small helpers shared across the repo workplane submodules.

use anyhow::{Context, Result};

pub(crate) fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

pub(crate) fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting runtime workplane integer to sqlite i64")
}

pub(crate) fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
