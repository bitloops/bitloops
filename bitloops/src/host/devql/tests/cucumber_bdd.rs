use super::cucumber_steps;
use super::cucumber_world::DevqlBddWorld;
use cucumber::{World as _, event::ScenarioFinished, writer::Stats as _};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn devql_bdd_features_pass() {
    let feature_dir = format!(
        "{}/src/host/devql/tests/features",
        env!("CARGO_MANIFEST_DIR")
    );
    let failed_scenarios = Arc::new(Mutex::new(Vec::new()));
    let failed_scenarios_for_after = Arc::clone(&failed_scenarios);

    let result = DevqlBddWorld::cucumber()
        .steps(cucumber_steps::collection())
        // Steps use `process_state_lock()` (see `with_cwd` / `enter_process_state`) and some
        // async steps hold that guard across `.await`. Default cucumber concurrency (64) lets
        // scenarios block each other on that mutex indefinitely.
        .max_concurrent_scenarios(1)
        .before(|_, _, scenario, world| {
            Box::pin(async move {
                world.reset();
                world.scenario_id = scenario.name.split_whitespace().next().map(str::to_string);
            })
        })
        .after(move |_, _, scenario, event, _| {
            let failed_scenarios = Arc::clone(&failed_scenarios_for_after);
            Box::pin(async move {
                if matches!(
                    event,
                    ScenarioFinished::BeforeHookFailed(_) | ScenarioFinished::StepFailed(_, _, _)
                ) {
                    failed_scenarios
                        .lock()
                        .expect("failed cucumber scenario list lock")
                        .push(scenario.name.clone());
                }
            })
        })
        .with_default_cli()
        .fail_on_skipped()
        .run(feature_dir)
        .await;
    let failed_scenarios = failed_scenarios
        .lock()
        .expect("failed cucumber scenario list lock")
        .join(", ");

    assert!(
        !result.execution_has_failed(),
        "cucumber suite reported failures{}",
        if failed_scenarios.is_empty() {
            String::new()
        } else {
            format!(": {failed_scenarios}")
        }
    );
    assert_eq!(result.skipped_steps(), 0, "cucumber suite skipped steps");
    assert_eq!(
        result.parsing_errors(),
        0,
        "cucumber suite had parse errors"
    );
}
