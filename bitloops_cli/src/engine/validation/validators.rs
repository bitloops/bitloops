use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ValidationError {}

fn is_path_safe_id(id: &str) -> bool {
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn validate_session_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() {
        return Err(ValidationError::new("session ID cannot be empty"));
    }
    if id.contains('/') || id.contains('\\') {
        return Err(ValidationError::new(format!(
            "invalid session ID {:?}: contains path separators",
            id
        )));
    }
    Ok(())
}

pub fn validate_tool_use_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() {
        return Ok(());
    }
    if !is_path_safe_id(id) {
        return Err(ValidationError::new(format!(
            "invalid tool use ID {:?}: must be alphanumeric with underscores/hyphens only",
            id
        )));
    }
    Ok(())
}

pub fn validate_agent_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() {
        return Ok(());
    }
    if !is_path_safe_id(id) {
        return Err(ValidationError::new(format!(
            "invalid agent ID {:?}: must be alphanumeric with underscores/hyphens only",
            id
        )));
    }
    Ok(())
}

pub fn validate_agent_session_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty() {
        return Err(ValidationError::new("agent session ID cannot be empty"));
    }
    if !is_path_safe_id(id) {
        return Err(ValidationError::new(format!(
            "invalid agent session ID {:?}: must be alphanumeric with underscores/hyphens only",
            id
        )));
    }
    Ok(())
}
