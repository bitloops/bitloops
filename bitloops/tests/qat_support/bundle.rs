use anyhow::{Error, Result};

pub struct BundleResult {
    suite: &'static str,
    rerun_hint: &'static str,
    result: Result<()>,
}

impl BundleResult {
    pub fn from_result(suite: &'static str, rerun_hint: &'static str, result: Result<()>) -> Self {
        Self {
            suite,
            rerun_hint,
            result,
        }
    }

    pub fn ok(suite: &'static str, rerun_hint: &'static str) -> Self {
        Self::from_result(suite, rerun_hint, Ok(()))
    }

    pub fn err(suite: &'static str, rerun_hint: &'static str, err: Error) -> Self {
        Self::from_result(suite, rerun_hint, Err(err))
    }
}

fn indent_block(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn combine_bundle_results(results: Vec<BundleResult>) -> Result<()> {
    let failures = results
        .into_iter()
        .filter_map(|entry| {
            entry
                .result
                .err()
                .map(|err| (entry.suite, entry.rerun_hint, err))
        })
        .collect::<Vec<_>>();

    if failures.is_empty() {
        return Ok(());
    }

    let mut message = format!(
        "QAT bundle reported {} failing suite{}:",
        failures.len(),
        if failures.len() == 1 { "" } else { "s" }
    );
    for (suite, rerun_hint, err) in failures {
        message.push_str(&format!("\n- {suite}"));
        message.push_str(&format!("\n  rerun: {rerun_hint}"));
        message.push_str("\n  details:");
        message.push('\n');
        message.push_str(&indent_block(&format!("{err:#}"), "    "));
    }

    Err(anyhow::anyhow!(message))
}
