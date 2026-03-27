use async_graphql::{Error, ErrorExtensions};

pub(crate) fn graphql_error(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(message.into()).extend_with(|_, extensions| {
        extensions.set("code", code);
    })
}

pub(crate) fn backend_error(message: impl Into<String>) -> Error {
    graphql_error("BACKEND_ERROR", message)
}

pub(crate) fn bad_user_input_error(message: impl Into<String>) -> Error {
    graphql_error("BAD_USER_INPUT", message)
}

pub(crate) fn bad_cursor_error(message: impl Into<String>) -> Error {
    graphql_error("BAD_CURSOR", message)
}
