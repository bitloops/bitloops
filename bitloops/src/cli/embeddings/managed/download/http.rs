//! HTTP request helpers, `Content-Range` parsing, and temporary path naming
//! for the managed runtime download pipeline.

use anyhow::{Context, Result};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, HeaderValue, USER_AGENT};
use std::env;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::DownloadByteRange;

/// A successfully parsed `Content-Range: bytes start-end/total` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParsedContentRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) total: u64,
}

/// Build a `GET` request configured with the managed runtime download
/// headers; callers attach the appropriate `Range` header themselves.
pub(crate) fn managed_download_request<'a>(
    client: &'a Client,
    url: &'a str,
    user_agent: &'a str,
) -> RequestBuilder {
    client
        .get(url)
        .header(ACCEPT, "application/octet-stream")
        .header(USER_AGENT, user_agent)
}

/// Parse a `Content-Range` header into its numeric components, returning
/// `None` for `*` totals or any other malformed input.
pub(crate) fn parse_content_range_header(header: &HeaderValue) -> Option<ParsedContentRange> {
    let value = header.to_str().ok()?.trim();
    let range = value.strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    if total == "*" {
        return None;
    }
    let (start, end) = span.split_once('-')?;
    Some(ParsedContentRange {
        start: start.parse().ok()?,
        end: end.parse().ok()?,
        total: total.parse().ok()?,
    })
}

/// Ensure that a 206 response actually returned the requested byte range and
/// reported the expected total length.
pub(crate) fn validate_content_range_header(
    header: Option<&HeaderValue>,
    expected_range: DownloadByteRange,
    expected_total: u64,
) -> Result<()> {
    let parsed = header
        .and_then(parse_content_range_header)
        .context("managed runtime range response is missing a valid Content-Range header")?;
    if parsed.start != expected_range.start
        || parsed.end != expected_range.end
        || parsed.total != expected_total
    {
        anyhow::bail!(
            "managed runtime range response returned bytes {}-{}/{} instead of {}-{}/{}",
            parsed.start,
            parsed.end,
            parsed.total,
            expected_range.start,
            expected_range.end,
            expected_total
        );
    }
    Ok(())
}

/// Build a deterministic-but-unique temporary path for a download or chunk
/// based on the asset label, the current process id and a nanosecond suffix.
pub(crate) fn temporary_download_path(asset_label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let safe_label: String = asset_label
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    env::temp_dir().join(format!(
        "bitloops-{safe_label}.{}.{}.download",
        std::process::id(),
        suffix
    ))
}
