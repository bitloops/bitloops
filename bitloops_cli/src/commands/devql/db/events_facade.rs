fn resolve_events_store_from_backends(
    backends: &DevqlBackendConfig,
) -> Result<Box<dyn EventsStore + Send + Sync>> {
    match backends.events.provider {
        EventsProvider::DuckDb => Ok(Box::new(DuckDbEventsStore::from_backend(&backends.events))),
        EventsProvider::ClickHouse => Ok(Box::new(ClickHouseEventsStore::from_backend(
            &backends.events,
        ))),
    }
}

fn resolve_events_store(cfg: &DevqlConfig) -> Result<Box<dyn EventsStore + Send + Sync>> {
    resolve_events_store_from_backends(&cfg.backends)
}

fn resolve_events_store_for_connection(
    cfg: &DevqlConnectionConfig,
) -> Result<Box<dyn EventsStore + Send + Sync>> {
    resolve_events_store_from_backends(&cfg.backends)
}

async fn events_store_ping(cfg: &DevqlConnectionConfig) -> Result<i32> {
    let store = resolve_events_store_for_connection(cfg)?;
    store.ping().await
}

async fn events_store_init_schema(cfg: &DevqlConfig) -> Result<()> {
    let store = resolve_events_store(cfg)?;
    store.init_schema().await
}

async fn events_store_existing_event_ids(cfg: &DevqlConfig, repo_id: &str) -> Result<HashSet<String>> {
    let store = resolve_events_store(cfg)?;
    store.existing_event_ids(repo_id.to_string()).await
}

async fn events_store_insert_checkpoint_event(
    cfg: &DevqlConfig,
    event: CheckpointEventWrite,
) -> Result<()> {
    let store = resolve_events_store(cfg)?;
    store.insert_checkpoint_event(event).await
}

async fn events_store_query_checkpoints(
    cfg: &DevqlConfig,
    query: EventsCheckpointQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_checkpoints(query).await
}

async fn events_store_query_telemetry(
    cfg: &DevqlConfig,
    query: EventsTelemetryQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_telemetry(query).await
}

async fn events_store_query_commit_shas(
    cfg: &DevqlConfig,
    query: EventsCommitShaQuery,
) -> Result<Vec<String>> {
    let store = resolve_events_store(cfg)?;
    store.query_commit_shas(query).await
}

async fn events_store_query_checkpoint_events(
    cfg: &DevqlConfig,
    query: EventsCheckpointHistoryQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_checkpoint_events(query).await
}

