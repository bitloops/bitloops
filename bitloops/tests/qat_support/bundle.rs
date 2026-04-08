use anyhow::Result;

pub fn combine_bundle_results(onboarding: Result<()>, devql_sync: Result<()>) -> Result<()> {
    match (onboarding, devql_sync) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) => Err(err),
        (Ok(()), Err(err)) => Err(err),
        (Err(onboarding_err), Err(devql_sync_err)) => Err(anyhow::anyhow!(
            "QAT bundle reported failures:\n- onboarding: {onboarding_err:#}\n- devql-sync: {devql_sync_err:#}"
        )),
    }
}
