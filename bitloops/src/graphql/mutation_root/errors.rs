use async_graphql::{Error, ErrorExtensions};

pub(super) fn operation_error(
    code: &'static str,
    kind: &'static str,
    operation: &'static str,
    error: impl std::fmt::Display,
) -> Error {
    Error::new(error.to_string()).extend_with(|_, extensions| {
        extensions.set("code", code);
        extensions.set("kind", kind);
        extensions.set("operation", operation);
    })
}
