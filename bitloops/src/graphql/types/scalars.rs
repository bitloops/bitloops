use async_graphql::{InputValueError, InputValueResult, ScalarType, Value};
use chrono::{DateTime, FixedOffset};

#[allow(dead_code)]
pub type JsonScalar = async_graphql::types::Json<serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTimeScalar(String);

impl DateTimeScalar {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_rfc3339(value: impl Into<String>) -> Result<Self, chrono::ParseError> {
        let value = value.into();
        let parsed = Self::parse_rfc3339(&value)?;
        Ok(Self(parsed.to_rfc3339()))
    }

    pub fn parse_rfc3339(value: &str) -> Result<DateTime<FixedOffset>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(value)
    }
}

#[async_graphql::Scalar(name = "DateTime")]
impl ScalarType for DateTimeScalar {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::String(raw) => DateTimeScalar::from_rfc3339(raw.clone())
                .map_err(|_| InputValueError::custom("expected an RFC 3339 timestamp")),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}
