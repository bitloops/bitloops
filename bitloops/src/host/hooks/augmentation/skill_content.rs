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
    fn using_devql_skill_mentions_search_lookup() {
        let body = using_devql_skill_body();
        assert!(body.contains("search"));
        assert!(body.contains("<natural-language request or approx symbol>"));
        assert!(body.contains("payLatr()"));
        assert!(body.contains("build invoice pdf"));
    }

    #[test]
    fn using_devql_skill_requires_summary_for_file_prompts() {
        let body = using_devql_skill_body();
        assert!(body.contains("If the input clearly identifies a specific file"));
        assert!(body.contains("start with `summary`."));
        assert!(body.contains("Once you have selected a file-level artefact"));
        assert!(body.contains("continue"));
        assert!(body.contains("from that summary."));
    }

    #[test]
    fn using_devql_skill_requires_search_followed_by_summary_for_natural_language_prompts() {
        let body = using_devql_skill_body();
        assert!(body.contains("If the input is natural language"));
        assert!(body.contains("always follow with `summary`"));
        assert!(body.contains("before expanding"));
        assert!(body.contains("stages."));
        assert!(body.contains("resolve concrete artefacts/files first"));
        assert!(body.contains("with `search`"));
    }

    #[test]
    fn using_devql_skill_forbids_summary_for_ambiguous_path_only_selectors() {
        let body = using_devql_skill_body();
        assert!(body.contains("Do not use `summary` for a `path` selector"));
        assert!(body.contains("unless the path clearly resolves"));
        assert!(body.contains("to a specific file."));
        assert!(body.contains("If a `path` selector may refer to a directory"));
        assert!(body.contains("another ambiguous scope"));
        assert!(body.contains("If the path resolves to a directory, use `entries(first: ...)`."));
        assert!(body.contains("Once you have selected a file-level artefact"));
    }

    #[test]
    fn using_devql_skill_explains_selector_routing() {
        let body = using_devql_skill_body();
        assert!(body.contains("Selector Routing"));
        assert!(body.contains("Do not pass the whole conversational prompt into `search`"));
        assert!(body.contains("For mixed prompts, try structured lookup first"));
        assert!(body.contains("help me understand the codebase"));
        assert!(body.contains("fuzzy and semantic are query styles"));
    }

    #[test]
    fn using_devql_skill_mentions_tests_stage_example() {
        let body = using_devql_skill_body();
        assert!(body.contains("tests { summary"));
        assert!(body.contains("coveringTests"));
    }
}
