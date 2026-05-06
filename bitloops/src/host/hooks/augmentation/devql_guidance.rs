pub fn build_session_bootstrap(surface_path: &str) -> String {
    format!(
        "<EXTREMELY_IMPORTANT>\n\
This repo has DevQL guidance available.\n\
The repo-local DevQL-capable guidance surface is installed at `{surface_path}`.\n\
When DevQL is available in this session, use DevQL first for repo-aware understanding questions.\n\
That includes questions about architecture, symbol lookup, path/line resolution, callers/usages/imports/tests/dependencies, and \"what does this repo do\".\n\
Start with the most specific selector available: `path`, `path + lines`, `symbolFqn`, or `search`.\n\
Use `searchMode: LEXICAL` for identifiers, literals, paths, and snippets.\n\
Keep `search` in the default `AUTO` mode for approximate or conceptual lookup, and inspect `searchBreakdown` when `AUTO` needs a wider net.\n\
Use `IDENTITY`, `CODE`, or `SUMMARY` only for advanced narrowing, then use `overview` once the selection is concrete.\n\
When DevQL returns a follow-up hint such as `expandHint`, use that hint to choose the next query.\n\
When DevQL returns `path`, `startLine`, and `endLine`, prefer bounded reads around those ranges before full-file reads, roughly 20 lines before `startLine` through 40 lines after `endLine`.\n\
If DevQL is unavailable in this session or returns no useful artefacts, fall back to targeted repo search or file reads.\n\
The repo-local guidance at `{surface_path}` is available as an optional reference if the concise rules above are insufficient.\n\
</EXTREMELY_IMPORTANT>"
    )
}

pub fn build_turn_guidance(surface_path: &str) -> String {
    format!(
        "Use DevQL first for this repo-aware request when it is available in this session. \
This applies even to repo-overview questions like \"what does this repo do?\". \
Prefer `path`, `path + lines`, or `symbolFqn` when the target is exact, and `search` when it is approximate or conceptual. \
Use `searchMode: LEXICAL` for identifiers, literals, and snippets; keep `AUTO` for conceptual lookup; inspect `searchBreakdown` when `AUTO` needs a wider net. \
Use `IDENTITY`, `CODE`, or `SUMMARY` only for intentional narrowing. \
Once the selection is concrete, use `overview` before broader repo search. \
If the response includes `expandHint`, use it to drive the next query. \
When DevQL returns `path`, `startLine`, and `endLine`, use bounded reads around those ranges before full-file reads, roughly 20 lines before `startLine` through 40 lines after `endLine`. \
If DevQL is unavailable or returns no useful artefacts, fall back to targeted repo search or file reads. \
Read `{surface_path}` before broad search."
    )
}

pub const DEVQL_REPO_UNDERSTANDING_TERMS: &[&str] = &[
    "what does this repo do",
    "understand",
    "explain",
    "architecture",
    "where is",
    "find",
    "inspect",
    "caller",
    "usage",
    "import",
    "dependency",
    "test covering",
];

pub const DEVQL_EXECUTION_TERMS: &[&str] = &[
    "fix ",
    "implement ",
    "edit ",
    "write ",
    "run ",
    "build ",
    "test ",
    "format ",
];

pub const DEVQL_CODE_REFERENCE_MARKERS: &[&str] = &["/", "::", "`", ":"];

pub fn prompt_warrants_devql(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let looks_like_code_reference = DEVQL_CODE_REFERENCE_MARKERS
        .iter()
        .any(|marker| prompt.contains(marker));
    let looks_like_edit_or_execution = DEVQL_EXECUTION_TERMS
        .iter()
        .any(|needle| lower.contains(needle));
    let looks_like_repo_understanding = DEVQL_REPO_UNDERSTANDING_TERMS
        .iter()
        .any(|needle| lower.contains(needle));

    (looks_like_code_reference || looks_like_repo_understanding) && !looks_like_edit_or_execution
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_session_bootstrap_mentions_search_modes_overview_and_response_hints() {
        let text = build_session_bootstrap(".opencode/skills/bitloops/devql-explore-first/SKILL.md");

        assert!(text.contains("This repo has DevQL guidance available."));
        assert!(text.contains("DevQL-capable guidance surface"));
        assert!(text.contains("When DevQL is available in this session"));
        assert!(text.contains("search"));
        assert!(text.contains("searchMode: LEXICAL"));
        assert!(text.contains("searchBreakdown"));
        assert!(text.contains("overview"));
        assert!(text.contains("expandHint"));
        assert!(text.contains("fall back to targeted repo search or file reads"));
        assert!(text.contains("startLine"));
        assert!(text.contains("endLine"));
        assert!(text.contains("bounded reads"));
        assert!(text.contains("20 lines before"));
        assert!(text.contains("40 lines after"));
        assert!(text.contains("full-file reads"));
        assert!(text.contains("available as an optional reference"));
        assert!(!text.contains("Read the repo-local guidance"));
        assert!(!text.contains("fuzzyName"));
        assert!(!text.contains("naturalLanguage"));
        assert!(!text.contains("semanticQuery"));
    }

    #[test]
    fn build_turn_guidance_mentions_search_modes_overview_and_skill_path() {
        let guidance = build_turn_guidance(".claude/skills/bitloops/devql-explore-first/SKILL.md");

        assert!(guidance.contains("when it is available in this session"));
        assert!(guidance.contains("what does this repo do?"));
        assert!(guidance.contains("path + lines"));
        assert!(guidance.contains("search"));
        assert!(guidance.contains("searchMode: LEXICAL"));
        assert!(guidance.contains("searchBreakdown"));
        assert!(guidance.contains("overview"));
        assert!(guidance.contains("expandHint"));
        assert!(guidance.contains(".claude/skills/bitloops/devql-explore-first/SKILL.md"));
        assert!(guidance.contains("fall back to targeted repo search or file reads"));
        assert!(guidance.contains("startLine"));
        assert!(guidance.contains("endLine"));
        assert!(guidance.contains("bounded reads"));
        assert!(guidance.contains("20 lines before"));
        assert!(guidance.contains("40 lines after"));
        assert!(guidance.contains("full-file reads"));
        assert!(!guidance.contains("fuzzyName"));
        assert!(!guidance.contains("naturalLanguage"));
        assert!(!guidance.contains("semanticQuery"));
    }

    #[test]
    fn prompt_warrants_devql_accepts_repo_overview_prompts_and_rejects_execution_prompts() {
        assert!(prompt_warrants_devql("What does this repo do?"));
        assert!(prompt_warrants_devql(
            "Help me understand src/payments/invoice.ts:42"
        ));
        assert!(!prompt_warrants_devql("Run cargo fmt"));
        assert!(!prompt_warrants_devql("Implement payment retry handling"));
    }
}
