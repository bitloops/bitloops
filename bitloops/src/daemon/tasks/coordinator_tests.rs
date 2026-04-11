use super::*;

#[tokio::test]
async fn receive_embeddings_bootstrap_outcome_waits_for_result_after_progress_channel_closes() {
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();

    tokio::spawn(async move {
        progress_tx
            .send(EmbeddingsBootstrapProgress {
                phase: EmbeddingsBootstrapPhase::WarmingProfile,
                message: Some("warming".to_string()),
                ..Default::default()
            })
            .expect("send bootstrap progress");
        drop(progress_tx);
        tokio::task::yield_now().await;
        result_tx
            .send(Ok(EmbeddingsBootstrapResult {
                version: Some("v0.1.2".to_string()),
                binary_path: None,
                cache_dir: None,
                runtime_name: None,
                model_name: Some("local_code".to_string()),
                freshly_installed: false,
                message: "ok".to_string(),
            }))
            .expect("send bootstrap result");
    });

    let mut seen_phases = Vec::new();
    let outcome = receive_embeddings_bootstrap_outcome(progress_rx, result_rx, |progress| {
        seen_phases.push(progress.phase);
        Ok(())
    })
    .await
    .expect("receive bootstrap outcome")
    .expect("bootstrap result");

    assert_eq!(seen_phases, vec![EmbeddingsBootstrapPhase::WarmingProfile]);
    assert_eq!(outcome.message, "ok");
}
