pub const USING_DEVQL_SKILL: &str = include_str!("skills/using-devql/SKILL.md");

pub fn using_devql_skill_body() -> &'static str {
    USING_DEVQL_SKILL
        .splitn(3, "---")
        .nth(2)
        .map(str::trim_start)
        .unwrap_or(USING_DEVQL_SKILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn using_devql_skill_body_strips_skill_frontmatter() {
        let body = using_devql_skill_body();
        assert!(body.contains("# Using DevQL"));
        assert!(!body.starts_with("---"));
        assert!(body.contains("bitloops devql query"));
    }

    #[test]
    fn using_devql_skill_mentions_running_bitloops_devql_outside_sandbox() {
        let body = using_devql_skill_body();
        assert!(body.contains("outside the sandbox"));
        assert!(body.contains("bitloops devql"));
    }

    #[test]
    fn using_devql_skill_centers_search_overview_and_selector_choice() {
        let body = using_devql_skill_body();
        assert!(body.contains("search"));
        assert!(body.contains("overview"));
        assert!(body.contains("Choosing The `by` Selector"));
        assert!(body.contains("path + lines"));
        assert!(body.contains("symbolFqn"));
        assert!(body.contains("expandHint"));
        assert!(body.contains("<distilled phrase or approximate symbol>"));
    }

    #[test]
    fn using_devql_skill_omits_stale_selector_names_and_summary_routing() {
        let body = using_devql_skill_body();
        assert!(!body.contains("fuzzyName"));
        assert!(!body.contains("naturalLanguage"));
        assert!(!body.contains("semanticQuery"));
        assert!(!body.contains("summary"));
        assert!(!body.contains("entries(first: ...)"));
        assert!(!body.contains("If the input clearly identifies a specific file"));
    }
}
