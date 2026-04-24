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
        Self::named_with(name, false, Vec::new())
    }

    fn named_with(name: &str, detected: bool, dirs: Vec<&str>) -> Self {
        Self {
            name: name.to_string(),
            agent_type: MOCK_AGENT_TYPE.to_string(),
            detected,
            dirs: dirs.into_iter().map(|dir| dir.to_string()).collect(),
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
    let cases = [
        (
            "registered agent lookup",
            vec![Box::new(MockAgent::named(MOCK_AGENT_NAME)) as Box<dyn Agent + Send + Sync>],
            MOCK_AGENT_NAME,
            Some(MOCK_AGENT_NAME),
        ),
        (
            "unknown agent lookup",
            vec![Box::new(MockAgent::named(MOCK_AGENT_NAME)) as Box<dyn Agent + Send + Sync>],
            "nonexistent-agent",
            None,
        ),
        (
            "sorted list",
            vec![
                Box::new(MockAgent::named("agent-b")) as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named("agent-a")) as Box<dyn Agent + Send + Sync>,
            ],
            "",
            None,
        ),
    ];

    for (name, agents, lookup_name, expected_name) in cases {
        let registry = AgentRegistry::new(agents);

        match (name, expected_name) {
            ("registered agent lookup", Some(expected_name)) => {
                let agent = registry
                    .get(lookup_name)
                    .expect("unexpected error retrieving registered agent");
                assert_eq!(agent.name(), expected_name);
            }
            ("unknown agent lookup", None) => {
                let err = match registry.get(lookup_name) {
                    Ok(_) => panic!("expected error for unknown agent"),
                    Err(err) => err,
                };
                assert!(
                    err.to_string().contains("unknown agent"),
                    "error should mention unknown agent"
                );
            }
            ("sorted list", None) => {
                assert_eq!(
                    registry.list(),
                    vec!["agent-a".to_string(), "agent-b".to_string()],
                    "list should be sorted"
                );
            }
            _ => unreachable!("unexpected case wiring"),
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn TestDetect() {
    let cases = [
        (
            "no detected agents",
            vec![
                Box::new(MockAgent::named("undetected")) as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named("also-undetected")) as Box<dyn Agent + Send + Sync>,
            ],
            None,
            Some(r#"available: ["also-undetected", "undetected"]"#),
        ),
        (
            "first detected agent follows sorted name order",
            vec![
                Box::new(MockAgent::named_with("zeta", true, vec![]))
                    as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named_with("alpha", true, vec![]))
                    as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named_with("middle", false, vec![]))
                    as Box<dyn Agent + Send + Sync>,
            ],
            Some("alpha"),
            None,
        ),
    ];

    for (name, agents, expected_detected, expected_error_fragment) in cases {
        let registry = AgentRegistry::new(agents);
        match expected_detected {
            Some(expected_name) => {
                let agent = registry.detect().expect("expected a detected agent");
                assert_eq!(agent.name(), expected_name, "case {name} mismatch");
                let detected_names = registry
                    .detect_all()
                    .into_iter()
                    .map(|agent| agent.name())
                    .collect::<Vec<_>>();
                assert_eq!(
                    detected_names,
                    vec![expected_name.to_string(), "zeta".to_string()],
                    "case {name} should return detected agents in sorted order"
                );
            }
            None => {
                let err = match registry.detect() {
                    Ok(_) => panic!("expected no detected agent"),
                    Err(err) => err,
                };
                assert!(
                    err.to_string().contains("no agent detected"),
                    "error should mention no agent detected"
                );
                if let Some(fragment) = expected_error_fragment {
                    assert!(
                        err.to_string().contains(fragment),
                        "case {name} should include the sorted available agents"
                    );
                }
            }
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn TestAgentNameConstants() {
    assert_eq!(AGENT_NAME_CLAUDE_CODE, "claude-code");
    assert_eq!(AGENT_NAME_COPILOT, "copilot");
    assert_eq!(AGENT_NAME_CODEX, "codex");
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
    let cases = [
        (
            "empty registry",
            Vec::<Box<dyn Agent + Send + Sync>>::new(),
            None,
        ),
        (
            "default agent present",
            vec![Box::new(MockAgent::named(DEFAULT_AGENT_NAME)) as Box<dyn Agent + Send + Sync>],
            Some(DEFAULT_AGENT_NAME),
        ),
        (
            "default agent not present",
            vec![Box::new(MockAgent::named("other")) as Box<dyn Agent + Send + Sync>],
            None,
        ),
    ];

    for (name, agents, expected_default) in cases {
        let registry = AgentRegistry::new(agents);
        let default = registry.default_agent().map(|agent| agent.name());

        match expected_default {
            Some(expected_name) => {
                assert_eq!(
                    default,
                    Some(expected_name.to_string()),
                    "case {name} should resolve the default agent"
                );
            }
            None => {
                assert!(
                    default.is_none(),
                    "case {name} should not resolve a default agent"
                );
            }
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn TestAllProtectedDirs() {
    let cases = [
        (
            "empty registry",
            Vec::<Box<dyn Agent + Send + Sync>>::new(),
            Vec::<&str>::new(),
        ),
        (
            "multiple agents with different dirs",
            vec![
                Box::new(MockAgent::named_with("agent-a", false, vec![".agent-a"]))
                    as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named_with(
                    "agent-b",
                    false,
                    vec![".agent-b", ".shared"],
                )) as Box<dyn Agent + Send + Sync>,
            ],
            vec![".agent-a", ".agent-b", ".shared"],
        ),
        (
            "duplicate dirs are deduplicated",
            vec![
                Box::new(MockAgent::named_with("agent-x", false, vec![".shared"]))
                    as Box<dyn Agent + Send + Sync>,
                Box::new(MockAgent::named_with("agent-y", false, vec![".shared"]))
                    as Box<dyn Agent + Send + Sync>,
            ],
            vec![".shared"],
        ),
    ];

    for (name, agents, expected_dirs) in cases {
        let registry = AgentRegistry::new(agents);
        let dirs = registry.all_protected_dirs();
        assert_eq!(
            dirs,
            expected_dirs
                .into_iter()
                .map(|dir| dir.to_string())
                .collect::<Vec<_>>(),
            "case {name} mismatch"
        );
    }
}

#[test]
fn builtin_registry_is_cached() {
    let first = AgentRegistry::builtin();
    let second = AgentRegistry::builtin();

    assert!(std::ptr::eq(first, second));
}
