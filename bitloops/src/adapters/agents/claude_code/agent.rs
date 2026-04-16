use std::path::Path;

use anyhow::Result;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_TYPE_CLAUDE_CODE, Agent, TokenCalculator, TokenUsage,
};

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

impl TokenCalculator for ClaudeCodeAgent {
    fn calculate_token_usage(&self, session_ref: &str, from_offset: usize) -> Result<TokenUsage> {
        super::transcript::calculate_token_usage_from_file(session_ref, from_offset)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::adapters::agents::{Agent, TokenCalculator};

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

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let transcript_path = dir.path().join("claude.jsonl");
        std::fs::write(
            &transcript_path,
            concat!(
                r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"Describe the repo"}}"#,
                "\n",
                r#"{"type":"assistant","uuid":"a1","message":{"id":"msg-1","role":"assistant","content":[{"type":"text","text":"It is an embeddings runtime."}],"usage":{"input_tokens":10,"cache_creation_input_tokens":4,"cache_read_input_tokens":6,"output_tokens":3}}}"#,
                "\n"
            ),
        )
        .expect("write transcript");

        let agent = ClaudeCodeAgent;
        let usage = agent
            .calculate_token_usage(transcript_path.to_string_lossy().as_ref(), 0)
            .expect("calculate token usage");

        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.cache_creation_tokens, 4);
        assert_eq!(usage.cache_read_tokens, 6);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.api_call_count, 1);
    }
}
