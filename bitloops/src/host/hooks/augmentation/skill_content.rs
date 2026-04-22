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
    fn using_devql_skill_mentions_fuzzy_name_lookup() {
        let body = using_devql_skill_body();
        assert!(body.contains("fuzzyName"));
        assert!(body.contains("<approx-symbol-name>"));
    }

    #[test]
    fn using_devql_skill_mentions_semantic_query_lookup() {
        let body = using_devql_skill_body();
        assert!(body.contains("semanticQuery"));
        assert!(body.contains("<natural-language request>"));
    }

    #[test]
    fn using_devql_skill_explains_selector_routing() {
        let body = using_devql_skill_body();
        assert!(body.contains("Selector Routing"));
        assert!(body.contains("Do not pass the whole conversational prompt into `semanticQuery`"));
        assert!(body.contains("For mixed prompts, try structured lookup first"));
        assert!(body.contains("help me understand the codebase"));
    }

    #[test]
    fn using_devql_skill_mentions_tests_stage_example() {
        let body = using_devql_skill_body();
        assert!(body.contains("tests { summary"));
        assert!(body.contains("coveringTests"));
    }

    #[test]
    fn using_devql_skill_mentions_semantic_query_lookup() {
        let body = using_devql_skill_body();
        assert!(body.contains("semanticQuery"));
        assert!(body.contains("<natural-language request>"));
    }

    #[test]
    fn using_devql_skill_explains_selector_routing() {
        let body = using_devql_skill_body();
        assert!(body.contains("Selector Routing"));
        assert!(body.contains("Do not pass the whole conversational prompt into `semanticQuery`"));
        assert!(body.contains("For mixed prompts, try structured lookup first"));
        assert!(body.contains("help me understand the codebase"));
    }

    #[test]
    fn using_devql_skill_mentions_tests_stage_example() {
        let body = using_devql_skill_body();
        assert!(body.contains("tests { summary"));
        assert!(body.contains("coveringTests"));
    }
}
