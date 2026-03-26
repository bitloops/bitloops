use super::types::EntryType;
use serde_json::Value;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct AgentSession {
    pub session_id: String,
    pub agent_name: String,
    pub repo_path: String,
    pub session_ref: String,
    pub start_time: SystemTime,
    pub native_data: Vec<u8>,
    pub export_data: Vec<u8>,
    pub modified_files: Vec<String>,
    pub new_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub entries: Vec<SessionEntry>,
}

impl Default for AgentSession {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            agent_name: String::new(),
            repo_path: String::new(),
            session_ref: String::new(),
            start_time: SystemTime::UNIX_EPOCH,
            native_data: Vec::new(),
            export_data: Vec::new(),
            modified_files: Vec::new(),
            new_files: Vec::new(),
            deleted_files: Vec::new(),
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub uuid: String,
    pub entry_type: EntryType,
    pub timestamp: SystemTime,
    pub content: String,
    pub tool_name: String,
    pub tool_input: Value,
    pub tool_output: Value,
    pub files_affected: Vec<String>,
}

impl Default for SessionEntry {
    fn default() -> Self {
        Self {
            uuid: String::new(),
            entry_type: EntryType::User,
            timestamp: SystemTime::UNIX_EPOCH,
            content: String::new(),
            tool_name: String::new(),
            tool_input: Value::Null,
            tool_output: Value::Null,
            files_affected: Vec::new(),
        }
    }
}

impl AgentSession {
    pub fn get_last_user_prompt(&self) -> String {
        for entry in self.entries.iter().rev() {
            if entry.entry_type == EntryType::User {
                return entry.content.clone();
            }
        }
        String::new()
    }

    pub fn get_last_assistant_response(&self) -> String {
        for entry in self.entries.iter().rev() {
            if entry.entry_type == EntryType::Assistant {
                return entry.content.clone();
            }
        }
        String::new()
    }

    pub fn truncate_at_uuid(&self, uuid: &str) -> AgentSession {
        if uuid.is_empty() {
            return self.clone();
        }

        let mut truncated = AgentSession {
            session_id: self.session_id.clone(),
            agent_name: self.agent_name.clone(),
            repo_path: self.repo_path.clone(),
            session_ref: self.session_ref.clone(),
            start_time: self.start_time,
            native_data: self.native_data.clone(),
            export_data: self.export_data.clone(),
            modified_files: Vec::new(),
            new_files: Vec::new(),
            deleted_files: Vec::new(),
            entries: Vec::new(),
        };

        for entry in &self.entries {
            truncated.entries.push(entry.clone());
            if entry.uuid == uuid {
                break;
            }
        }

        let mut seen_files = std::collections::HashSet::new();
        for entry in &truncated.entries {
            for file in &entry.files_affected {
                if seen_files.insert(file.clone()) {
                    truncated.modified_files.push(file.clone());
                }
            }
        }

        truncated
    }

    pub fn find_tool_result_uuid(&self, tool_use_id: &str) -> Option<String> {
        for entry in &self.entries {
            if entry.entry_type == EntryType::Tool && entry.uuid == tool_use_id {
                return Some(entry.uuid.clone());
            }
        }
        None
    }
}
