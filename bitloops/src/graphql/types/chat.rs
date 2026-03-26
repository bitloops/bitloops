use async_graphql::{Enum, SimpleObject};

use super::{DateTimeScalar, JsonScalar};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

impl ChatRole {
    pub(crate) fn from_raw(value: Option<&str>) -> Self {
        let normalised = value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase().replace('-', "_"))
            .unwrap_or_default();

        if normalised.is_empty() {
            return Self::Assistant;
        }
        if normalised == "human" || normalised.contains("user") {
            return Self::User;
        }
        if normalised.contains("system") {
            return Self::System;
        }
        if normalised.contains("tool") || normalised.contains("function") {
            return Self::Tool;
        }

        Self::Assistant
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct ChatEntry {
    pub session_id: String,
    pub agent: String,
    pub timestamp: DateTimeScalar,
    pub role: ChatRole,
    pub content: String,
    pub metadata: Option<JsonScalar>,
    #[graphql(skip)]
    pub(crate) cursor: String,
}

impl ChatEntry {
    pub fn cursor(&self) -> String {
        self.cursor.clone()
    }
}
