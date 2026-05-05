use crate::graphql::{bad_user_input_error, graphql_error};

pub(super) fn internal_config_error(error: anyhow::Error) -> async_graphql::Error {
    graphql_error("internal", format!("{error:#}"))
}

pub(super) fn map_snapshot_error(error: anyhow::Error) -> async_graphql::Error {
    if error.to_string().contains("No such file") {
        return bad_user_input_error(format!("{error:#}"));
    }
    internal_config_error(error)
}
