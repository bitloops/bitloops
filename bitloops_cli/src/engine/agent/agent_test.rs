use super::*;
use serde_json::json;
use std::collections::HashMap;
use std::io::Cursor;

const MOCK_AGENT_NAME: &str = "mock";
const MOCK_AGENT_TYPE: &str = "Mock Agent";
const MINIMAL_AGENT_NAME: &str = "minimal";
const MINIMAL_AGENT_TYPE: &str = "Minimal Agent";

#[derive(Default)]
struct MockAgent;

impl Agent for MockAgent {
    fn name(&self) -> String {
        MOCK_AGENT_NAME.to_string()
    }

    fn agent_type(&self) -> String {
        MOCK_AGENT_TYPE.to_string()
    }

    fn description(&self) -> String {
        "Mock agent for testing".to_string()
    }

    fn is_preview(&self) -> bool {
        false
    }

    fn detect_presence(&self) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn get_session_id(&self, _input: &HookInput) -> String {
        String::new()
    }

    fn protected_dirs(&self) -> Vec<String> {
        Vec::new()
    }

    fn hook_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn read_transcript(&self, _session_ref: &str) -> anyhow::Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn chunk_transcript(&self, content: &[u8], _max_size: usize) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(vec![content.to_vec()])
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
        Ok(chunks.concat())
    }

    fn get_session_dir(&self, _repo_path: &str) -> anyhow::Result<String> {
        Ok(String::new())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        format!("{session_dir}/{agent_session_id}.jsonl")
    }

    fn read_session(&self, _input: &HookInput) -> anyhow::Result<Option<AgentSession>> {
        Ok(None)
    }

    fn write_session(&self, _session: &AgentSession) -> anyhow::Result<()> {
        Ok(())
    }

    fn format_resume_command(&self, _session_id: &str) -> String {
        String::new()
    }
}

#[derive(Default)]
struct MockHookSupport {
    inner: MockAgent,
}

impl Agent for MockHookSupport {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn agent_type(&self) -> String {
        self.inner.agent_type()
    }
}

impl HookSupport for MockHookSupport {}

#[derive(Default)]
struct MockFileWatcher {
    inner: MockAgent,
}

impl Agent for MockFileWatcher {
    fn name(&self) -> String {
        self.inner.name()
    }

    fn agent_type(&self) -> String {
        self.inner.agent_type()
    }
}

impl FileWatcher for MockFileWatcher {}

struct MinimalAgent;

impl Agent for MinimalAgent {
    fn name(&self) -> String {
        MINIMAL_AGENT_NAME.to_string()
    }

    fn agent_type(&self) -> String {
        MINIMAL_AGENT_TYPE.to_string()
    }
}

struct MinimalHookSupport;

impl Agent for MinimalHookSupport {
    fn name(&self) -> String {
        MINIMAL_AGENT_NAME.to_string()
    }

    fn agent_type(&self) -> String {
        MINIMAL_AGENT_TYPE.to_string()
    }
}

impl HookSupport for MinimalHookSupport {}

struct MinimalFileWatcher;

impl Agent for MinimalFileWatcher {
    fn name(&self) -> String {
        MINIMAL_AGENT_NAME.to_string()
    }

    fn agent_type(&self) -> String {
        MINIMAL_AGENT_TYPE.to_string()
    }
}

impl FileWatcher for MinimalFileWatcher {}

#[test]
#[allow(non_snake_case)]
fn TestAgentInterfaceCompliance() {
    {
        let agent: Box<dyn Agent + Send + Sync> = Box::new(MockAgent);
        assert_eq!(
            agent.name(),
            MOCK_AGENT_NAME,
            "expected name to match mock agent name"
        );
    }

    {
        let hook_support: Box<dyn HookSupport + Send + Sync> = Box::new(MockHookSupport::default());
        let agent: &dyn Agent = hook_support.as_ref();
        assert_eq!(
            agent.name(),
            MOCK_AGENT_NAME,
            "HookSupport should satisfy Agent"
        );
    }

    {
        let file_watcher: Box<dyn FileWatcher + Send + Sync> = Box::new(MockFileWatcher::default());
        let agent: &dyn Agent = file_watcher.as_ref();
        assert_eq!(
            agent.name(),
            MOCK_AGENT_NAME,
            "FileWatcher should satisfy Agent"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestHookTypeConstants() {
    let cases = [
        (HookType::SessionStart, "session_start"),
        (HookType::UserPromptSubmit, "user_prompt_submit"),
        (HookType::Stop, "stop"),
        (HookType::PreToolUse, "pre_tool_use"),
        (HookType::PostToolUse, "post_tool_use"),
    ];

    for (hook_type, expected) in cases {
        assert_eq!(
            hook_type.as_str(),
            expected,
            "expected hook type string to match"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestEntryTypeConstants() {
    let cases = [
        (EntryType::User, "user"),
        (EntryType::Assistant, "assistant"),
        (EntryType::Tool, "tool"),
        (EntryType::System, "system"),
    ];

    for (entry_type, expected) in cases {
        assert_eq!(
            entry_type.as_str(),
            expected,
            "expected entry type string to match"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestHookInputStructure() {
    let mut raw_data = HashMap::new();
    raw_data.insert("extra".to_string(), json!("data"));

    let input = HookInput {
        hook_type: HookType::PreToolUse,
        session_id: "test-session".to_string(),
        raw_data,
        ..HookInput::default()
    };

    assert_eq!(input.hook_type, HookType::PreToolUse);
    assert_eq!(input.session_id, "test-session");
    assert_eq!(input.hook_type.as_str(), "pre_tool_use");
}

#[test]
#[allow(non_snake_case)]
fn TestSessionChangeStructure() {
    let change = SessionChange {
        session_id: "test-session".to_string(),
        event_type: HookType::SessionStart,
        ..SessionChange::default()
    };

    assert_eq!(change.session_id, "test-session");
    assert_eq!(change.event_type, HookType::SessionStart);
    assert_eq!(change.event_type.as_str(), "session_start");
}

#[test]
#[allow(non_snake_case)]
fn TestAgentDefaultMethodsAreSafeNoOps() {
    let agent = MinimalAgent;
    let mut stdin = Cursor::new(br#"{"ignored":true}"#);

    assert_eq!(agent.description(), "TODO");
    assert!(agent.is_preview());
    assert!(
        !agent
            .detect_presence()
            .expect("presence check should succeed")
    );
    assert_eq!(agent.get_session_id(&HookInput::default()), "");
    assert!(agent.protected_dirs().is_empty());
    assert!(agent.hook_names().is_empty());
    assert!(
        agent
            .parse_hook_event("stop", &mut stdin)
            .expect("hook parsing should succeed")
            .is_none()
    );
    assert!(
        agent
            .read_transcript("session-ref")
            .expect("transcript read should succeed")
            .is_empty()
    );
    assert_eq!(
        agent
            .chunk_transcript(b"hello", 1)
            .expect("chunking should succeed"),
        vec![b"hello".to_vec()]
    );
    assert_eq!(
        agent
            .reassemble_transcript(&[b"he".to_vec(), b"llo".to_vec()])
            .expect("reassembly should succeed"),
        b"hello".to_vec()
    );
    assert_eq!(
        agent
            .get_session_dir("/tmp/repo")
            .expect("session dir lookup should succeed"),
        ""
    );
    assert_eq!(
        agent.resolve_session_file("/tmp/sessions", "abc123"),
        "/tmp/sessions/abc123.jsonl"
    );
    assert!(
        agent
            .read_session(&HookInput::default())
            .expect("session read should succeed")
            .is_none()
    );
    agent
        .write_session(&AgentSession::default())
        .expect("session write should succeed");
    assert_eq!(agent.format_resume_command("abc123"), "");
}

#[test]
#[allow(non_snake_case)]
fn TestHookSupportDefaultMethodsAreSafeNoOps() {
    let agent = MinimalHookSupport;

    assert_eq!(
        agent
            .install_hooks(false, false)
            .expect("hook install should succeed"),
        0
    );
    agent
        .uninstall_hooks()
        .expect("hook uninstall should succeed");
    assert!(!agent.are_hooks_installed());
}

#[test]
#[allow(non_snake_case)]
fn TestFileWatcherDefaultMethodsAreSafeNoOps() {
    let watcher = MinimalFileWatcher;

    assert!(
        watcher
            .get_watch_paths()
            .expect("watch path lookup should succeed")
            .is_empty()
    );
    assert!(
        watcher
            .on_file_change("src/main.rs")
            .expect("file change handler should succeed")
            .is_none()
    );
}
