use crate::host::checkpoints::checkpoint_id::is_valid_checkpoint_id;

use super::super::dto::ApiError;
use super::super::{ApiPage, CommitCheckpointQuery};

fn normalize_optional_query(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn require_query_value(
    field: &str,
    value: Option<String>,
) -> std::result::Result<String, ApiError> {
    normalize_optional_query(value)
        .ok_or_else(|| ApiError::bad_request(format!("{field} is required")))
}

pub(super) fn parse_optional_unix_seconds(
    field: &str,
    value: Option<String>,
) -> std::result::Result<Option<i64>, ApiError> {
    let Some(raw) = normalize_optional_query(value) else {
        return Ok(None);
    };
    raw.parse::<i64>()
        .map(Some)
        .map_err(|_| ApiError::bad_request(format!("invalid {field}; expected unix seconds")))
}

pub(super) fn parse_optional_usize(
    field: &str,
    value: Option<String>,
) -> std::result::Result<Option<usize>, ApiError> {
    let Some(raw) = normalize_optional_query(value) else {
        return Ok(None);
    };
    raw.parse::<usize>().map(Some).map_err(|_| {
        ApiError::bad_request(format!("invalid {field}; expected non-negative integer"))
    })
}

pub(super) fn normalize_checkpoint_id(
    checkpoint_id: String,
) -> std::result::Result<String, ApiError> {
    let normalized = checkpoint_id.trim().to_ascii_lowercase();
    if !is_valid_checkpoint_id(&normalized) {
        return Err(ApiError::bad_request(
            "invalid checkpoint_id; expected 12 lowercase hex characters",
        ));
    }
    Ok(normalized)
}

pub(super) fn validate_time_window(
    from: Option<i64>,
    to: Option<i64>,
) -> std::result::Result<(), ApiError> {
    if let (Some(from), Some(to)) = (from, to)
        && from > to
    {
        return Err(ApiError::bad_request(
            "from must be less than or equal to to",
        ));
    }
    Ok(())
}

pub(super) fn parse_commit_checkpoint_filter(
    branch: Option<String>,
    from: Option<String>,
    to: Option<String>,
    user: Option<String>,
    agent: Option<String>,
) -> std::result::Result<CommitCheckpointQuery, ApiError> {
    let branch = require_query_value("branch", branch)?;
    let from_unix = parse_optional_unix_seconds("from", from)?;
    let to_unix = parse_optional_unix_seconds("to", to)?;
    validate_time_window(from_unix, to_unix)?;

    Ok(CommitCheckpointQuery {
        branch,
        from_unix,
        to_unix,
        user: normalize_optional_query(user),
        agent: normalize_optional_query(agent),
        page: ApiPage::default(),
    })
}
