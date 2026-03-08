use super::registry::AgentRegistry;
use super::*;

const MOCK_AGENT_NAME: &str = "mock";
const MOCK_AGENT_TYPE: &str = "Mock Agent";

#[derive(Clone)]
struct MockAgent {
    name: String,
    agent_type: String,
    detected: bool,
    dirs: Vec<String>,
}

impl MockAgent {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            agent_type: MOCK_AGENT_TYPE.to_string(),
            detected: false,
            dirs: Vec::new(),
        }
    }
}

impl Agent for MockAgent {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn agent_type(&self) -> String {
        self.agent_type.clone()
    }

    fn detect_presence(&self) -> anyhow::Result<bool> {
        Ok(self.detected)
    }

    fn protected_dirs(&self) -> Vec<String> {
        self.dirs.clone()
    }
}

#[test]
#[allow(non_snake_case)]
fn TestRegistryOperations() {
    // Test: register and get an agent
    {
        let registry = AgentRegistry::new(vec![Box::new(MockAgent::named(MOCK_AGENT_NAME))]);

        let agent = registry
            .get(MOCK_AGENT_NAME)
            .expect("unexpected error retrieving registered agent");
        assert_eq!(agent.name(), MOCK_AGENT_NAME);
    }

    // Test: get unknown agent returns error
    {
        let registry = AgentRegistry::new(vec![Box::new(MockAgent::named(MOCK_AGENT_NAME))]);

        let err = match registry.get("nonexistent-agent") {
            Ok(_) => panic!("expected error for unknown agent"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("unknown agent"),
            "error should mention unknown agent"
        );
    }

    // Test: list is sorted
    {
        let registry = AgentRegistry::new(vec![
            Box::new(MockAgent::named("agent-b")),
            Box::new(MockAgent::named("agent-a")),
        ]);

        let names = registry.list();
        assert_eq!(
            names,
            vec!["agent-a".to_string(), "agent-b".to_string()],
            "list should be sorted"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestDetect() {
    // Test: detect with no detected agents returns error
    {
        let registry = AgentRegistry::new(vec![Box::new(MockAgent::named("undetected"))]);
        let err = match registry.detect() {
            Ok(_) => panic!("expected no detected agent"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("no agent detected"),
            "error should mention no agent detected"
        );
    }

    // Test: detect with a detected agent returns it
    {
        let registry = AgentRegistry::new(vec![Box::new(MockAgent {
            name: "detectable".to_string(),
            agent_type: MOCK_AGENT_TYPE.to_string(),
            detected: true,
            dirs: Vec::new(),
        })]);

        let agent = registry.detect().expect("expected a detected agent");
        assert_eq!(agent.name(), "detectable");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestAgentNameConstants() {
    assert_eq!(AGENT_NAME_CLAUDE_CODE, "claude-code");
    assert_eq!(AGENT_NAME_CURSOR, "cursor");
    assert_eq!(AGENT_NAME_GEMINI, "gemini");
    assert_eq!(AGENT_NAME_OPEN_CODE, "opencode");
}

#[test]
#[allow(non_snake_case)]
fn TestDefaultAgentName() {
    assert_eq!(DEFAULT_AGENT_NAME, AGENT_NAME_CLAUDE_CODE);
}

#[test]
#[allow(non_snake_case)]
fn TestDefault() {
    // Empty registry: default is None
    {
        let registry = AgentRegistry::new(vec![]);
        let default = registry.default_agent();
        assert!(
            default.is_none(),
            "default should be None when not registered"
        );
    }

    // Registry with default agent: returns Some
    {
        let registry = AgentRegistry::new(vec![Box::new(MockAgent::named(DEFAULT_AGENT_NAME))]);
        let default = registry.default_agent();
        assert!(
            default.is_some(),
            "default should be Some after registering default"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestAllProtectedDirs() {
    // Test: empty registry
    {
        let registry = AgentRegistry::new(vec![]);
        let dirs = registry.all_protected_dirs();
        assert!(dirs.is_empty(), "expected empty dirs for empty registry");
    }

    // Test: multiple agents with different dirs
    {
        let registry = AgentRegistry::new(vec![
            Box::new(MockAgent {
                name: "agent-a".to_string(),
                agent_type: MOCK_AGENT_TYPE.to_string(),
                detected: false,
                dirs: vec![".agent-a".to_string()],
            }),
            Box::new(MockAgent {
                name: "agent-b".to_string(),
                agent_type: MOCK_AGENT_TYPE.to_string(),
                detected: false,
                dirs: vec![".agent-b".to_string(), ".shared".to_string()],
            }),
        ]);

        let dirs = registry.all_protected_dirs();
        assert_eq!(
            dirs,
            vec![
                ".agent-a".to_string(),
                ".agent-b".to_string(),
                ".shared".to_string()
            ],
            "all_protected_dirs should return sorted union"
        );
    }

    // Test: duplicate dirs are deduplicated
    {
        let registry = AgentRegistry::new(vec![
            Box::new(MockAgent {
                name: "agent-x".to_string(),
                agent_type: MOCK_AGENT_TYPE.to_string(),
                detected: false,
                dirs: vec![".shared".to_string()],
            }),
            Box::new(MockAgent {
                name: "agent-y".to_string(),
                agent_type: MOCK_AGENT_TYPE.to_string(),
                detected: false,
                dirs: vec![".shared".to_string()],
            }),
        ]);

        let dirs = registry.all_protected_dirs();
        assert_eq!(dirs.len(), 1, "duplicate dirs should be deduplicated");
    }
}
