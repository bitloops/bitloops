use super::*;

pub fn codecity_legends() -> CodeCityLegends {
    CodeCityLegends {
        arc_kinds: vec![
            CodeCityArcKindLegend {
                kind: CodeCityArcKind::Dependency,
                label: "Dependency".to_string(),
                default_visible: false,
                description: "Normal file dependency, returned when selecting a building."
                    .to_string(),
            },
            CodeCityArcKindLegend {
                kind: CodeCityArcKind::Violation,
                label: "Architecture violation".to_string(),
                default_visible: true,
                description: "Dependency that contradicts the detected architecture.".to_string(),
            },
            CodeCityArcKindLegend {
                kind: CodeCityArcKind::CrossBoundary,
                label: "Cross-boundary dependency".to_string(),
                default_visible: true,
                description: "Aggregated dependency between CodeCity boundaries.".to_string(),
            },
            CodeCityArcKindLegend {
                kind: CodeCityArcKind::Cycle,
                label: "Cycle".to_string(),
                default_visible: false,
                description: "Dependency participating in a detected cycle.".to_string(),
            },
            CodeCityArcKindLegend {
                kind: CodeCityArcKind::Bridge,
                label: "Bridge".to_string(),
                default_visible: false,
                description: "Dependency involving a broad bridge file.".to_string(),
            },
        ],
        violation_rules: violation_rule_legends(),
        severities: vec![
            CodeCitySeverityLegend {
                severity: CodeCityViolationSeverity::High,
                label: "High".to_string(),
                description: "Likely architecture break that should be reviewed.".to_string(),
            },
            CodeCitySeverityLegend {
                severity: CodeCityViolationSeverity::Medium,
                label: "Medium".to_string(),
                description: "Structural warning that may need design attention.".to_string(),
            },
            CodeCitySeverityLegend {
                severity: CodeCityViolationSeverity::Low,
                label: "Low".to_string(),
                description: "Minor diagnostic signal.".to_string(),
            },
            CodeCitySeverityLegend {
                severity: CodeCityViolationSeverity::Info,
                label: "Info".to_string(),
                description: "Contextual diagnostic, not a violation.".to_string(),
            },
        ],
    }
}

fn violation_rule_legends() -> Vec<CodeCityViolationRuleLegend> {
    vec![
        rule_legend(
            CodeCityViolationRule::LayeredUpwardDependency,
            CodeCityViolationPattern::Layered,
            CodeCityViolationSeverity::High,
            "A lower/deeper layer depends on an upper/user-facing layer.",
        ),
        rule_legend(
            CodeCityViolationRule::LayeredSkippedLayer,
            CodeCityViolationPattern::Layered,
            CodeCityViolationSeverity::Medium,
            "A dependency jumps over an intermediate layer.",
        ),
        rule_legend(
            CodeCityViolationRule::HexagonalCoreImportsPeriphery,
            CodeCityViolationPattern::Hexagonal,
            CodeCityViolationSeverity::High,
            "Core code imports an adapter/periphery file.",
        ),
        rule_legend(
            CodeCityViolationRule::HexagonalCoreImportsExternal,
            CodeCityViolationPattern::Hexagonal,
            CodeCityViolationSeverity::High,
            "Core code imports a conservatively detected external package.",
        ),
        rule_legend(
            CodeCityViolationRule::HexagonalApplicationImportsEdge,
            CodeCityViolationPattern::Hexagonal,
            CodeCityViolationSeverity::Medium,
            "Application code imports an edge/API file.",
        ),
        rule_legend(
            CodeCityViolationRule::ModularInternalCrossModuleDependency,
            CodeCityViolationPattern::Modular,
            CodeCityViolationSeverity::High,
            "A module depends directly on another module's internal file.",
        ),
        rule_legend(
            CodeCityViolationRule::ModularBroadBridgeFile,
            CodeCityViolationPattern::Modular,
            CodeCityViolationSeverity::Medium,
            "A file bridges many modules without being an explicit facade.",
        ),
        rule_legend(
            CodeCityViolationRule::EventDrivenDirectPeerDependency,
            CodeCityViolationPattern::EventDriven,
            CodeCityViolationSeverity::Medium,
            "Event-driven peers are coupled directly instead of through messages.",
        ),
        rule_legend(
            CodeCityViolationRule::CrossBoundaryCycle,
            CodeCityViolationPattern::Cycle,
            CodeCityViolationSeverity::High,
            "Boundaries participate in a macro dependency cycle.",
        ),
        rule_legend(
            CodeCityViolationRule::CrossBoundaryHighCoupling,
            CodeCityViolationPattern::CrossBoundary,
            CodeCityViolationSeverity::Medium,
            "A cross-boundary dependency is unusually heavy.",
        ),
    ]
}

fn rule_legend(
    rule: CodeCityViolationRule,
    pattern: CodeCityViolationPattern,
    severity: CodeCityViolationSeverity,
    description: &str,
) -> CodeCityViolationRuleLegend {
    CodeCityViolationRuleLegend {
        rule,
        pattern,
        severity,
        description: description.to_string(),
    }
}
