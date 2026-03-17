use super::Agent;
use super::adapters::{
    AgentAdapterCapability, AgentAdapterCompatibility, AgentAdapterDescriptor,
    AgentAdapterRegistration, AgentAdapterRegistry,
};
use anyhow::Result;
use std::path::Path;

const NO_ALIASES: &[&str] = &[];
const ALIAS_ALPHA: &[&str] = &["alpha-cli"];
const ALIAS_BETA: &[&str] = &["beta-cli"];
const SHARED_ALIAS: &[&str] = &["shared-cli"];
const BASE_CAPABILITIES: &[AgentAdapterCapability] = &[AgentAdapterCapability::HookInstallation];

struct TestAgent {
    name: &'static str,
    agent_type: &'static str,
    dirs: &'static [&'static str],
}

impl Agent for TestAgent {
    fn name(&self) -> String {
        self.name.to_string()
    }

    fn agent_type(&self) -> String {
        self.agent_type.to_string()
    }

    fn protected_dirs(&self) -> Vec<String> {
        self.dirs.iter().map(|dir| (*dir).to_string()).collect()
    }
}

fn create_alpha_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(TestAgent {
        name: "alpha",
        agent_type: "alpha-type",
        dirs: &[".alpha", ".shared"],
    })
}

fn create_beta_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(TestAgent {
        name: "beta",
        agent_type: "beta-type",
        dirs: &[".beta", ".shared"],
    })
}

fn detect_true(_: &Path) -> bool {
    true
}

fn detect_false(_: &Path) -> bool {
    false
}

fn hooks_true(_: &Path) -> bool {
    true
}

fn hooks_false(_: &Path) -> bool {
    false
}

fn install_noop(_: &Path, _: bool, _: bool) -> Result<usize> {
    Ok(0)
}

fn uninstall_noop(_: &Path) -> Result<()> {
    Ok(())
}

fn resume_alpha(session_id: &str) -> String {
    if session_id.trim().is_empty() {
        "alpha".to_string()
    } else {
        format!("alpha --resume {session_id}")
    }
}

fn resume_beta(session_id: &str) -> String {
    if session_id.trim().is_empty() {
        "beta".to_string()
    } else {
        format!("beta --resume {session_id}")
    }
}

#[derive(Clone, Copy)]
struct AdapterCallbacks {
    create_agent: fn() -> Box<dyn Agent + Send + Sync>,
    detect_project_presence: fn(&Path) -> bool,
    hooks_installed: fn(&Path) -> bool,
    format_resume_command: fn(&str) -> String,
}

const ALPHA_CALLBACKS: AdapterCallbacks = AdapterCallbacks {
    create_agent: create_alpha_agent,
    detect_project_presence: detect_true,
    hooks_installed: hooks_true,
    format_resume_command: resume_alpha,
};

const BETA_CALLBACKS: AdapterCallbacks = AdapterCallbacks {
    create_agent: create_beta_agent,
    detect_project_presence: detect_false,
    hooks_installed: hooks_false,
    format_resume_command: resume_beta,
};

fn make_registration(
    id: &'static str,
    display_name: &'static str,
    agent_type: &'static str,
    aliases: &'static [&'static str],
    is_default: bool,
    callbacks: AdapterCallbacks,
) -> AgentAdapterRegistration {
    AgentAdapterRegistration::new(
        AgentAdapterDescriptor {
            id,
            display_name,
            agent_type,
            aliases,
            is_default,
            capabilities: BASE_CAPABILITIES,
            compatibility: AgentAdapterCompatibility::phase1(),
        },
        callbacks.create_agent,
        callbacks.detect_project_presence,
        callbacks.hooks_installed,
        install_noop,
        uninstall_noop,
        callbacks.format_resume_command,
    )
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryRejectsInvalidRegistrations() {
    let err = match AgentAdapterRegistry::new(vec![]) {
        Ok(_) => panic!("expected empty registration error"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("at least one adapter registration is required")
    );

    let duplicate_id = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
        ),
        make_registration(
            "alpha",
            "Alpha Duplicate",
            "alpha-type-2",
            NO_ALIASES,
            false,
            BETA_CALLBACKS,
        ),
    ]) {
        Ok(_) => panic!("expected duplicate id error"),
        Err(err) => err,
    };
    assert!(duplicate_id.to_string().contains("duplicate adapter id"));

    let duplicate_agent_type = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "shared-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
        ),
        make_registration(
            "beta",
            "Beta",
            "shared-type",
            NO_ALIASES,
            false,
            BETA_CALLBACKS,
        ),
    ]) {
        Ok(_) => panic!("expected duplicate type error"),
        Err(err) => err,
    };
    assert!(
        duplicate_agent_type
            .to_string()
            .contains("duplicate adapter agent type")
    );

    let alias_collision = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            SHARED_ALIAS,
            true,
            ALPHA_CALLBACKS,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            SHARED_ALIAS,
            false,
            BETA_CALLBACKS,
        ),
    ]) {
        Ok(_) => panic!("expected alias collision"),
        Err(err) => err,
    };
    assert!(alias_collision.to_string().contains("alias collision"));

    let multiple_defaults = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            NO_ALIASES,
            true,
            BETA_CALLBACKS,
        ),
    ]) {
        Ok(_) => panic!("expected multiple defaults"),
        Err(err) => err,
    };
    assert!(
        multiple_defaults
            .to_string()
            .contains("multiple default adapters configured")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryResolvesAliasesAndCollectsReadiness() {
    let registry = AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            ALIAS_ALPHA,
            true,
            ALPHA_CALLBACKS,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            ALIAS_BETA,
            false,
            BETA_CALLBACKS,
        ),
    ])
    .expect("valid adapter registry");

    assert_eq!(
        registry.available_agents(),
        vec!["alpha".to_string(), "beta".to_string()]
    );
    assert_eq!(registry.default_agent_name(), "alpha");
    assert_eq!(
        registry.normalise_agent_name("alpha-cli").expect("alias"),
        "alpha"
    );
    assert_eq!(
        registry.normalise_agent_name("BETA-CLI").expect("alias"),
        "beta"
    );
    assert_eq!(
        registry
            .format_resume_command("alpha", "session-123")
            .expect("resume command"),
        "alpha --resume session-123"
    );

    let repo = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        registry.detect_project_agents(repo.path()),
        vec!["alpha".to_string()]
    );
    assert_eq!(
        registry.installed_agents(repo.path()),
        vec!["alpha".to_string()]
    );

    let readiness = registry.collect_readiness(repo.path());
    assert_eq!(readiness.len(), 2);
    assert!(readiness[0].project_detected);
    assert!(readiness[0].hooks_installed);
    assert!(readiness[0].compatibility_ok);
    assert!(!readiness[1].project_detected);
    assert!(!readiness[1].hooks_installed);
    assert!(readiness[1].compatibility_ok);

    assert_eq!(
        registry.all_protected_dirs(),
        vec![
            ".alpha".to_string(),
            ".beta".to_string(),
            ".shared".to_string(),
        ]
    );
}

#[test]
#[allow(non_snake_case)]
fn TestBuiltinAdapterRegistrySupportsCanonicalResolution() {
    let registry = AgentAdapterRegistry::builtin();

    assert_eq!(registry.default_agent_name(), "claude-code");
    assert_eq!(
        registry.normalise_agent_name("copilot-cli").expect("alias"),
        "copilot"
    );
    assert_eq!(
        registry.normalise_agent_name("gemini").expect("alias"),
        "gemini-cli"
    );
    assert_eq!(
        registry.normalise_agent_name("open-code").expect("alias"),
        "opencode"
    );
}
