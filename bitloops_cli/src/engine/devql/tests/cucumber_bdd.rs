use super::cucumber_world::DevqlBddWorld;
use super::cucumber_steps;
use cucumber::{World as _, writer::Stats as _};

#[tokio::test]
async fn devql_bdd_features_pass() {
    let feature_dir = format!(
        "{}/src/engine/devql/tests/features",
        env!("CARGO_MANIFEST_DIR")
    );

    let result = DevqlBddWorld::cucumber()
        .steps(cucumber_steps::collection())
        .before(|_, _, scenario, world| {
            Box::pin(async move {
                world.reset();
                world.scenario_id = scenario
                    .name
                    .split_whitespace()
                    .next()
                    .map(str::to_string);
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
    assert_eq!(result.parsing_errors(), 0, "cucumber suite had parse errors");
}
