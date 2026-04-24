use anyhow::{Context, Result, bail};

pub(super) fn validate_analytics_sql(sql: &str) -> Result<String> {
    let stripped = strip_sql_literals_and_comments(sql);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        bail!("analytics SQL cannot be empty");
    }

    let without_trailing_semicolon = trimmed.trim_end_matches(';').trim_end();
    if without_trailing_semicolon.contains(';') {
        bail!("analytics SQL accepts exactly one statement");
    }

    let mut tokens = sql_keyword_tokens(without_trailing_semicolon).into_iter();
    let first = tokens
        .next()
        .context("analytics SQL must start with SELECT or WITH")?;
    if first != "SELECT" && first != "WITH" {
        bail!("analytics SQL must start with SELECT or WITH");
    }

    const BLOCKED: &[&str] = &[
        "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "ATTACH", "DETACH", "COPY",
        "PRAGMA", "CALL", "INSTALL", "LOAD", "EXPORT", "IMPORT",
    ];
    let blocked = sql_keyword_tokens(without_trailing_semicolon)
        .into_iter()
        .find(|token| BLOCKED.contains(&token.as_str()));
    if let Some(keyword) = blocked {
        bail!("analytics SQL rejects `{keyword}`; only read-only queries are allowed");
    }

    Ok(sql.trim().trim_end_matches(';').trim().to_string())
}

fn strip_sql_literals_and_comments(input: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Normal,
        SingleQuote,
        DoubleQuote,
        LineComment,
        BlockComment,
    }

    let chars = input.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(chars.len());
    let mut index = 0usize;
    let mut state = State::Normal;

    while index < chars.len() {
        match state {
            State::Normal => {
                if chars[index] == '\'' {
                    out.push(' ');
                    state = State::SingleQuote;
                    index += 1;
                } else if chars[index] == '"' {
                    out.push(' ');
                    state = State::DoubleQuote;
                    index += 1;
                } else if chars[index] == '-' && chars.get(index + 1) == Some(&'-') {
                    out.push(' ');
                    out.push(' ');
                    state = State::LineComment;
                    index += 2;
                } else if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                    out.push(' ');
                    out.push(' ');
                    state = State::BlockComment;
                    index += 2;
                } else {
                    out.push(chars[index]);
                    index += 1;
                }
            }
            State::SingleQuote => {
                if chars[index] == '\'' {
                    if chars.get(index + 1) == Some(&'\'') {
                        out.push(' ');
                        out.push(' ');
                        index += 2;
                    } else {
                        out.push(' ');
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
            State::DoubleQuote => {
                if chars[index] == '"' {
                    if chars.get(index + 1) == Some(&'"') {
                        out.push(' ');
                        out.push(' ');
                        index += 2;
                    } else {
                        out.push(' ');
                        state = State::Normal;
                        index += 1;
                    }
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
            State::LineComment => {
                if chars[index] == '\n' {
                    out.push('\n');
                    state = State::Normal;
                } else {
                    out.push(' ');
                }
                index += 1;
            }
            State::BlockComment => {
                if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                    out.push(' ');
                    out.push(' ');
                    state = State::Normal;
                    index += 2;
                } else {
                    out.push(if chars[index] == '\n' { '\n' } else { ' ' });
                    index += 1;
                }
            }
        }
    }

    out
}

fn sql_keyword_tokens(input: &str) -> Vec<String> {
    input
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_uppercase())
        .collect::<Vec<_>>()
}
