use crate::graphql::types::DateTimeScalar;
use anyhow::{Context, Result, bail};
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

#[allow(dead_code)]
pub(super) fn parse_payload(value: Option<&Value>) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed = serde_json::from_str(trimmed)
                .unwrap_or_else(|_| Value::String(trimmed.to_string()));
            Ok(Some(parsed))
        }
        Some(other) => Ok(Some(other.clone())),
    }
}

pub(super) fn parse_event_time(raw: &str) -> Result<DateTimeScalar> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("event time is empty");
    }

    if let Ok(value) = DateTimeScalar::from_rfc3339(trimmed.to_string()) {
        return Ok(value);
    }

    for pattern in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, pattern) {
            let timestamp = Utc.from_utc_datetime(&naive).to_rfc3339();
            return DateTimeScalar::from_rfc3339(timestamp)
                .context("formatting event timestamp as RFC 3339");
        }
    }

    if let Ok(unix_seconds) = trimmed.parse::<i64>()
        && let Some(timestamp) = Utc.timestamp_opt(unix_seconds, 0).single()
    {
        return DateTimeScalar::from_rfc3339(timestamp.to_rfc3339())
            .context("formatting unix event timestamp as RFC 3339");
    }

    bail!("unsupported event timestamp `{trimmed}`")
}

#[allow(dead_code)]
pub(super) fn escape_like_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
