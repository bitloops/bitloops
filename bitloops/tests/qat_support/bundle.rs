use anyhow::{Error, Result};

pub struct BundleResult {
    suite: &'static str,
    result: Result<()>,
}

impl BundleResult {
    pub fn from_result(suite: &'static str, result: Result<()>) -> Self {
        Self { suite, result }
    }

    pub fn ok(suite: &'static str) -> Self {
        Self::from_result(suite, Ok(()))
    }

    pub fn err(suite: &'static str, err: Error) -> Self {
        Self::from_result(suite, Err(err))
    }
}

pub fn combine_bundle_results(results: Vec<BundleResult>) -> Result<()> {
    let mut failures = results
        .into_iter()
        .filter_map(|entry| entry.result.err().map(|err| (entry.suite, err)))
        .collect::<Vec<_>>();

    match failures.len() {
        0 => Ok(()),
        1 => Err(failures.pop().expect("single failure must exist").1),
        _ => {
            let mut message = String::from("QAT bundle reported failures:");
            for (suite, err) in failures {
                message.push_str(&format!("\n- {suite}: {err:#}"));
            }
            Err(anyhow::anyhow!(message))
        }
    }
}
