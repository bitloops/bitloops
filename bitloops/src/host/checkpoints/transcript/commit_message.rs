use crate::utils::strings;

pub const DEFAULT_COMMIT_MESSAGE: &str = "Claude Code session updates";

pub fn clean_prompt_for_commit(prompt: &str) -> String {
    let prefixes = [
        "Can you ",
        "can you ",
        "Please ",
        "please ",
        "Let's ",
        "let's ",
        "Could you ",
        "could you ",
        "Would you ",
        "would you ",
        "I want you to ",
        "I'd like you to ",
        "I need you to ",
    ];

    let mut cleaned = prompt.to_string();

    loop {
        let mut found = false;
        for prefix in prefixes {
            if cleaned.starts_with(prefix) {
                cleaned = cleaned[prefix.len()..].to_string();
                found = true;
                break;
            }
        }

        if !found {
            break;
        }
    }

    if let Some(without_q) = cleaned.strip_suffix('?') {
        cleaned = without_q.to_string();
    }
    cleaned = cleaned.trim().to_string();

    cleaned = strings::truncate_runes(&cleaned, 72, "");
    cleaned = cleaned.trim().to_string();
    strings::capitalize_first(&cleaned)
}

pub fn generate_commit_message(original_prompt: &str) -> String {
    if !original_prompt.is_empty() {
        let cleaned = clean_prompt_for_commit(original_prompt);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }

    DEFAULT_COMMIT_MESSAGE.to_string()
}

#[cfg(test)]
mod tests {
    use super::{clean_prompt_for_commit, generate_commit_message};

    #[test]
    fn test_clean_prompt_for_commit() {
        let tests = [
            (
                "removes 'Can you ' prefix",
                "Can you fix the bug",
                "Fix the bug",
            ),
            (
                "removes 'can you ' prefix (lowercase)",
                "can you fix the bug",
                "Fix the bug",
            ),
            (
                "removes 'Please ' prefix",
                "Please update the readme",
                "Update the readme",
            ),
            (
                "removes 'please ' prefix (lowercase)",
                "please update the readme",
                "Update the readme",
            ),
            (
                "removes 'Let's ' prefix",
                "Let's add a new feature",
                "Add a new feature",
            ),
            (
                "removes 'let's ' prefix (lowercase)",
                "let's add a new feature",
                "Add a new feature",
            ),
            (
                "removes 'Could you ' prefix",
                "Could you refactor this code",
                "Refactor this code",
            ),
            (
                "removes 'could you ' prefix (lowercase)",
                "could you refactor this code",
                "Refactor this code",
            ),
            (
                "removes 'Would you ' prefix",
                "Would you implement the API",
                "Implement the API",
            ),
            (
                "removes 'would you ' prefix (lowercase)",
                "would you implement the API",
                "Implement the API",
            ),
            (
                "removes 'I want you to ' prefix",
                "I want you to create a test file",
                "Create a test file",
            ),
            (
                "removes 'I'd like you to ' prefix",
                "I'd like you to optimize the query",
                "Optimize the query",
            ),
            (
                "removes 'I need you to ' prefix",
                "I need you to fix the auth flow",
                "Fix the auth flow",
            ),
            (
                "removes chained prefixes 'Can you please '",
                "Can you please fix the bug",
                "Fix the bug",
            ),
            (
                "removes chained prefixes 'Could you please '",
                "Could you please update the config",
                "Update the config",
            ),
            (
                "removes chained prefixes 'Would you please '",
                "Would you please add tests",
                "Add tests",
            ),
            (
                "removes trailing question mark",
                "Can you fix this?",
                "Fix this",
            ),
            (
                "handles prompt with no question mark",
                "Fix the authentication issue",
                "Fix the authentication issue",
            ),
            ("capitalizes first letter", "fix the bug", "Fix the bug"),
            (
                "preserves already capitalized",
                "Fix the bug",
                "Fix the bug",
            ),
            (
                "capitalizes after prefix removal",
                "please fix the bug",
                "Fix the bug",
            ),
            (
                "truncates at 72 characters and trims trailing space",
                "This is a very long prompt that exceeds the seventy two character limit and should be truncated",
                "This is a very long prompt that exceeds the seventy two character limit",
            ),
            (
                "keeps prompts under 72 chars intact",
                "Short prompt",
                "Short prompt",
            ),
            (
                "exactly 72 characters stays intact",
                "This is exactly seventy two characters long which is the maximum allowed",
                "This is exactly seventy two characters long which is the maximum allowed",
            ),
            ("handles empty string", "", ""),
            ("handles whitespace only", "   ", ""),
            (
                "trims leading/trailing whitespace",
                "  fix the bug  ",
                "Fix the bug",
            ),
            (
                "handles single character after prefix removal",
                "Can you x",
                "X",
            ),
            ("handles prefix that leaves empty string", "Can you ", ""),
            ("handles only question mark after prefix", "Can you ?", ""),
        ];

        for (name, input, expected) in tests {
            let result = clean_prompt_for_commit(input);
            assert_eq!(result, expected, "{name}");
        }
    }

    #[test]
    fn test_generate_commit_message() {
        let tests = [
            (
                "returns cleaned prompt",
                "Can you fix the login bug?",
                "Fix the login bug",
            ),
            (
                "returns default for empty prompt",
                "",
                "Claude Code session updates",
            ),
            (
                "returns default when cleaned prompt is empty",
                "Can you ?",
                "Claude Code session updates",
            ),
            (
                "returns default for whitespace only prompt",
                "   ",
                "Claude Code session updates",
            ),
            (
                "handles direct command prompt",
                "Add unit tests for the auth module",
                "Add unit tests for the auth module",
            ),
            (
                "handles polite request",
                "Please refactor the database connection handling",
                "Refactor the database connection handling",
            ),
        ];

        for (name, prompt, expected) in tests {
            let result = generate_commit_message(prompt);
            assert_eq!(result, expected, "{name}");
        }
    }
}
