pub const USING_DEVQL_SKILL: &str = include_str!("skills/using-devql/SKILL.md");
pub const DEVQL_EXPLORE_FIRST_SKILL: &str = include_str!("skills/devql-explore-first/SKILL.md");

pub fn using_devql_skill_body() -> &'static str {
    USING_DEVQL_SKILL
        .splitn(3, "---")
        .nth(2)
        .map(str::trim_start)
        .unwrap_or(USING_DEVQL_SKILL)
}

pub fn devql_explore_first_skill_body() -> &'static str {
    DEVQL_EXPLORE_FIRST_SKILL
        .splitn(3, "---")
        .nth(2)
        .map(str::trim_start)
        .unwrap_or(DEVQL_EXPLORE_FIRST_SKILL)
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
    fn using_devql_skill_centers_search_modes_overview_and_selector_choice() {
        let body = using_devql_skill_body();
        assert!(body.contains("search"));
        assert!(body.contains("searchMode: LEXICAL"));
        assert!(body.contains("searchBreakdown(first: 3)"));
        assert!(body.contains("overview"));
        assert!(body.contains("Choosing The `by` Selector"));
        assert!(body.contains("path + lines"));
        assert!(body.contains("symbolFqn"));
        assert!(body.contains("expandHint"));
        assert!(body.contains("<distilled conceptual phrase>"));
    }

    #[test]
    fn using_devql_skill_prefers_bounded_reads_after_line_ranges() {
        let body = using_devql_skill_body();
        assert!(body.contains("startLine"));
        assert!(body.contains("endLine"));
        assert!(body.contains("bounded read"));
        assert!(body.contains("20 lines before"));
        assert!(body.contains("40 lines after"));
        assert!(body.contains("full-file read"));
    }

    #[test]
    fn using_devql_skill_omits_stale_selector_names_and_old_routing_terms() {
        let body = using_devql_skill_body();
        assert!(!body.contains("fuzzyName"));
        assert!(!body.contains("naturalLanguage"));
        assert!(!body.contains("semanticQuery"));
        assert!(!body.contains("entries(first: ...)"));
        assert!(!body.contains("If the input clearly identifies a specific file"));
    }

    #[test]
    fn devql_explore_first_skill_is_minimal_and_operational() {
        let skill = DEVQL_EXPLORE_FIRST_SKILL;
        let body = devql_explore_first_skill_body();

        assert!(skill.contains("name: devql-explore-first"));
        assert!(skill.contains("before using rg, grep, sed, find"));
        assert!(body.contains("<VERY_IMPORTANT>"));
        assert!(body.contains("primary discovery tool"));
        assert!(body.contains("not"));
        assert!(body.contains("one-time preflight"));
        assert!(body.contains("whenever locating symbols"));
        assert!(body.contains("</VERY_IMPORTANT>"));
        assert!(!body.contains("Reading this `SKILL.md` does not count as DevQL usage"));
        assert!(body.contains("while DevQL can answer"));
        assert!(body.contains("lookup"));
        assert!(body.contains("read bounded ranges returned by DevQL"));
        assert!(body.contains("fall back when DevQL fails"));
        assert!(body.contains("Do not run `bitloops devql --help`"));
        assert!(body.contains("bitloops devql query '{ selectArtefacts"));
        assert!(body.contains("searchMode: LEXICAL"));
        assert!(body.contains("default `AUTO`"));
        assert!(body.contains("search: \"<short behavior phrase or task keywords>\""));
        assert!(body.contains("single concrete identifier"));
        assert!(body.contains("multiple related terms"));
        assert!(body.contains("one exact"));
        assert!(body.contains("anchor"));
        assert!(
            body.contains(
                "search: \"<single identifier, literal, path fragment, or short snippet>\""
            )
        );
        assert!(body.contains("symbolFqn"));
        assert!(body.contains("path"));
        assert!(body.contains("lines"));
        assert!(body.contains("Fuzzy symbol-name lookup is included in the lexical lane"));
        assert!(body.contains("path symbolFqn canonicalKind startLine endLine score"));
        assert!(body.contains("do not duplicate the same search"));
        assert!(body.contains("for each new exploration question"));
        assert!(body.contains("query DevQL again"));
        assert!(body.contains("use at most 3 DevQL calls"));
        assert!(body.contains("each bounded source-read phase"));
        assert!(body.contains("50 lines before and after"));
        assert!(!body.contains("searchBreakdown"));
        assert!(!body.contains("overview"));
        assert!(!body.contains("IDENTITY"));
        assert!(!body.contains("CODE"));
        assert!(!body.contains("SUMMARY"));
        assert!(!body.contains("summary"));

        let conceptual_idx = body
            .find("search: \"<short behavior phrase or task keywords>\"")
            .expect("conceptual search template");
        let lexical_idx = body
            .find(
                "search: \"<single identifier, literal, path fragment, or short snippet>\", searchMode: LEXICAL",
            )
            .expect("lexical search template");
        assert!(conceptual_idx < lexical_idx);
    }
}
