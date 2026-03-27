use super::cucumber_steps;
use super::cucumber_world::DevqlBddWorld;
use cucumber::{World as _, writer::Stats as _};

#[tokio::test]
async fn devql_bdd_features_pass() {
    let feature_dir = format!(
        "{}/src/host/devql/tests/features",
        env!("CARGO_MANIFEST_DIR")
    );

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
        .with_default_cli()
        .fail_on_skipped()
        .run(feature_dir)
        .await;

    assert!(
        !result.execution_has_failed(),
        "cucumber suite reported failures"
    );
    assert_eq!(result.skipped_steps(), 0, "cucumber suite skipped steps");
    assert_eq!(
        result.parsing_errors(),
        0,
        "cucumber suite had parse errors"
    );
}
