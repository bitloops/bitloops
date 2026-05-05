use async_graphql::Result;

use super::errors::operation_error;

pub(super) fn require_non_empty_input(
    value: String,
    field: &'static str,
    operation: &'static str,
) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("{field} must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

pub(super) fn normalise_optional_input(
    value: Option<String>,
    field: &'static str,
    operation: &'static str,
) -> Result<Option<String>> {
    value
        .map(|value| require_non_empty_input(value, field, operation))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_non_empty_input_trims_and_rejects_blank_values() {
        let value =
            require_non_empty_input("  hello  ".to_string(), "field", "operation").expect("trim");
        assert_eq!(value, "hello");

        let err = require_non_empty_input("   ".to_string(), "field", "operation")
            .expect_err("blank input should fail");
        let message = err.message.clone();
        assert!(message.contains("field must not be empty"));
    }
}
