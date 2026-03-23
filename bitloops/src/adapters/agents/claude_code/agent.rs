use std::path::Path;

use crate::adapters::agents::{AGENT_NAME_CLAUDE_CODE, AGENT_TYPE_CLAUDE_CODE, Agent};

#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeCodeAgent;

impl Agent for ClaudeCodeAgent {
    fn name(&self) -> String {
        AGENT_NAME_CLAUDE_CODE.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_CLAUDE_CODE.to_string()
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        Path::new(session_dir)
            .join(format!("{agent_session_id}.jsonl"))
            .to_string_lossy()
            .to_string()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".claude".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::adapters::agents::Agent;

    use super::ClaudeCodeAgent;

    #[test]
    #[allow(non_snake_case)]
    fn TestResolveSessionFile() {
        let agent = ClaudeCodeAgent;
        let got = agent.resolve_session_file("/home/user/.claude/projects/foo", "abc-123-def");
        let expected = Path::new("/home/user/.claude/projects/foo")
            .join("abc-123-def.jsonl")
            .to_string_lossy()
            .to_string();
        assert_eq!(got, expected);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestProtectedDirs() {
        let agent = ClaudeCodeAgent;
        assert_eq!(agent.protected_dirs(), vec![".claude".to_string()]);
    }
}
