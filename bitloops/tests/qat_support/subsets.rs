use super::runner::Suite;

pub const TAG_AGENT_SMOKE: &str = "@agent_smoke";
pub const TAG_DEVELOP_GATE: &str = "@develop_gate";
pub const DEVELOP_GATE_TAG_EXPR: &str = TAG_DEVELOP_GATE;
pub const DEVELOP_GATE_RERUN_ALIAS: &str = "cargo qat-develop-gate";

pub const DEVELOP_GATE_SUITES: &[Suite] = &[
    Suite::AgentSmoke,
    Suite::DevqlSync,
    Suite::Devql,
    Suite::DevqlIngest,
    Suite::AgentsCheckpoints,
];
