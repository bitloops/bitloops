use async_graphql::dataloader::{DataLoader, HashMapCache, Loader};
use async_graphql::extensions::{
    Extension, ExtensionContext, ExtensionFactory, NextPrepareRequest,
};
use async_graphql::{Request, ServerResult};
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::types::{Artefact, Commit, DependencyEdge, DepsDirection, DepsFilterInput, EdgeKind};
use super::{DevqlGraphqlContext, ResolverScope};

#[derive(Debug, Clone, Default)]
pub(crate) struct LoaderMetrics {
    artefact_by_id_batches: Arc<AtomicUsize>,
    outgoing_edge_batches: Arc<AtomicUsize>,
    incoming_edge_batches: Arc<AtomicUsize>,
    commit_by_sha_batches: Arc<AtomicUsize>,
}

impl LoaderMetrics {
    fn record_artefact_batch(&self) {
        self.artefact_by_id_batches.fetch_add(1, Ordering::Relaxed);
    }

    fn record_outgoing_edge_batch(&self) {
        self.outgoing_edge_batches.fetch_add(1, Ordering::Relaxed);
    }

    fn record_incoming_edge_batch(&self) {
        self.incoming_edge_batches.fetch_add(1, Ordering::Relaxed);
    }

    fn record_commit_batch(&self) {
        self.commit_by_sha_batches.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> LoaderMetricsSnapshot {
        LoaderMetricsSnapshot {
            artefact_by_id_batches: self.artefact_by_id_batches.load(Ordering::Relaxed),
            outgoing_edge_batches: self.outgoing_edge_batches.load(Ordering::Relaxed),
            incoming_edge_batches: self.incoming_edge_batches.load(Ordering::Relaxed),
            commit_by_sha_batches: self.commit_by_sha_batches.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoaderMetricsSnapshot {
    pub(crate) artefact_by_id_batches: usize,
    pub(crate) outgoing_edge_batches: usize,
    pub(crate) incoming_edge_batches: usize,
    pub(crate) commit_by_sha_batches: usize,
}

pub(crate) struct LoaderRegistryExtension;

impl ExtensionFactory for LoaderRegistryExtension {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(LoaderRegistryRequestExtension)
    }
}

struct LoaderRegistryRequestExtension;

#[async_graphql::async_trait::async_trait]
impl Extension for LoaderRegistryRequestExtension {
    async fn prepare_request(
        &self,
        ctx: &ExtensionContext<'_>,
        request: Request,
        next: NextPrepareRequest<'_>,
    ) -> ServerResult<Request> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        next.run(ctx, request.data(DataLoaders::new(context))).await
    }
}

pub(crate) struct DataLoaders {
    artefact_by_id: DataLoader<ArtefactByIdLoader, HashMapCache>,
    outgoing_edges_by_artefact: DataLoader<EdgesByArtefactLoader, HashMapCache>,
    incoming_edges_by_artefact: DataLoader<EdgesByArtefactLoader, HashMapCache>,
    commit_by_sha: DataLoader<CommitByShaLoader, HashMapCache>,
}

impl DataLoaders {
    fn new(context: &DevqlGraphqlContext) -> Self {
        Self {
            artefact_by_id: DataLoader::with_cache(
                ArtefactByIdLoader {
                    context: context.clone(),
                },
                tokio::spawn,
                HashMapCache::default(),
            ),
            outgoing_edges_by_artefact: DataLoader::with_cache(
                EdgesByArtefactLoader {
                    context: context.clone(),
                    direction: EdgeLoaderDirection::Outgoing,
                },
                tokio::spawn,
                HashMapCache::default(),
            ),
            incoming_edges_by_artefact: DataLoader::with_cache(
                EdgesByArtefactLoader {
                    context: context.clone(),
                    direction: EdgeLoaderDirection::Incoming,
                },
                tokio::spawn,
                HashMapCache::default(),
            ),
            commit_by_sha: DataLoader::with_cache(
                CommitByShaLoader {
                    context: context.clone(),
                },
                tokio::spawn,
                HashMapCache::default(),
            ),
        }
    }

    pub(crate) async fn load_artefact_by_id(
        &self,
        artefact_id: &str,
        scope: &ResolverScope,
    ) -> Result<Option<Artefact>, String> {
        self.artefact_by_id
            .load_one(ArtefactBatchKey::new(artefact_id, scope))
            .await
    }

    pub(crate) async fn load_outgoing_edges(
        &self,
        artefact_id: &str,
        filter: Option<DepsFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<DependencyEdge>, String> {
        Ok(self
            .outgoing_edges_by_artefact
            .load_one(EdgeBatchKey::new(
                artefact_id,
                filter.unwrap_or_default(),
                scope,
            ))
            .await?
            .unwrap_or_default())
    }

    pub(crate) async fn load_incoming_edges(
        &self,
        artefact_id: &str,
        filter: Option<DepsFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<DependencyEdge>, String> {
        Ok(self
            .incoming_edges_by_artefact
            .load_one(EdgeBatchKey::new(
                artefact_id,
                filter.unwrap_or_default(),
                scope,
            ))
            .await?
            .unwrap_or_default())
    }

    pub(crate) async fn load_commit_by_sha(
        &self,
        commit_sha: &str,
    ) -> Result<Option<Commit>, String> {
        self.commit_by_sha.load_one(commit_sha.to_string()).await
    }
}

struct ArtefactByIdLoader {
    context: DevqlGraphqlContext,
}

impl Loader<ArtefactBatchKey> for ArtefactByIdLoader {
    type Value = Artefact;
    type Error = String;

    async fn load(
        &self,
        keys: &[ArtefactBatchKey],
    ) -> Result<HashMap<ArtefactBatchKey, Self::Value>, Self::Error> {
        self.context.loader_metrics().record_artefact_batch();
        let mut grouped_ids = HashMap::<ResolverScope, Vec<String>>::new();
        for key in keys {
            grouped_ids
                .entry(key.scope.clone())
                .or_default()
                .push(key.artefact_id.clone());
        }

        let mut artefacts_by_key = HashMap::new();
        for (scope, artefact_ids) in grouped_ids {
            let artefacts = self
                .context
                .load_artefacts_by_ids(&artefact_ids, &scope)
                .await
                .map_err(|err| format!("{err:#}"))?;
            for artefact_id in artefact_ids {
                if let Some(artefact) = artefacts.get(&artefact_id).cloned() {
                    artefacts_by_key.insert(
                        ArtefactBatchKey::from_scope(artefact_id, scope.clone()),
                        artefact,
                    );
                }
            }
        }

        Ok(artefacts_by_key)
    }
}

#[derive(Debug, Clone, Copy)]
enum EdgeLoaderDirection {
    Outgoing,
    Incoming,
}

impl EdgeLoaderDirection {
    fn as_deps_direction(self) -> DepsDirection {
        match self {
            Self::Outgoing => DepsDirection::Out,
            Self::Incoming => DepsDirection::In,
        }
    }

    fn record_batch(self, metrics: &LoaderMetrics) {
        match self {
            Self::Outgoing => metrics.record_outgoing_edge_batch(),
            Self::Incoming => metrics.record_incoming_edge_batch(),
        }
    }
}

struct EdgesByArtefactLoader {
    context: DevqlGraphqlContext,
    direction: EdgeLoaderDirection,
}

impl Loader<EdgeBatchKey> for EdgesByArtefactLoader {
    type Value = Vec<DependencyEdge>;
    type Error = String;

    async fn load(
        &self,
        keys: &[EdgeBatchKey],
    ) -> Result<HashMap<EdgeBatchKey, Self::Value>, Self::Error> {
        let mut rows_by_key = HashMap::new();
        let mut grouped_ids = HashMap::<EdgeFilterKey, Vec<String>>::new();

        for key in keys {
            grouped_ids
                .entry(key.filter_key())
                .or_default()
                .push(key.artefact_id.clone());
        }

        for (filter_key, artefact_ids) in grouped_ids {
            self.direction.record_batch(self.context.loader_metrics());
            let filter = filter_key.as_filter(self.direction.as_deps_direction());
            let edges_by_artefact = self
                .context
                .load_dependency_edges_by_artefact_ids(
                    &artefact_ids,
                    self.direction.as_deps_direction(),
                    filter,
                    &filter_key.scope,
                )
                .await
                .map_err(|err| format!("{err:#}"))?;

            for artefact_id in artefact_ids {
                rows_by_key.insert(
                    EdgeBatchKey::from_filter_key(artefact_id.clone(), &filter_key),
                    edges_by_artefact
                        .get(&artefact_id)
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }

        Ok(rows_by_key)
    }
}

struct CommitByShaLoader {
    context: DevqlGraphqlContext,
}

impl Loader<String> for CommitByShaLoader {
    type Value = Commit;
    type Error = String;

    async fn load(&self, keys: &[String]) -> Result<HashMap<String, Self::Value>, Self::Error> {
        self.context.loader_metrics().record_commit_batch();
        self.context
            .load_commits_by_shas(keys)
            .await
            .map_err(|err| format!("{err:#}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ArtefactBatchKey {
    artefact_id: String,
    scope: ResolverScope,
}

impl ArtefactBatchKey {
    fn new(artefact_id: &str, scope: &ResolverScope) -> Self {
        Self {
            artefact_id: artefact_id.to_string(),
            scope: scope.clone(),
        }
    }

    fn from_scope(artefact_id: String, scope: ResolverScope) -> Self {
        Self { artefact_id, scope }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeBatchKey {
    artefact_id: String,
    kind: Option<EdgeKind>,
    include_unresolved: bool,
    scope: ResolverScope,
}

impl EdgeBatchKey {
    fn new(artefact_id: &str, filter: DepsFilterInput, scope: &ResolverScope) -> Self {
        Self {
            artefact_id: artefact_id.to_string(),
            kind: filter.kind,
            include_unresolved: filter.include_unresolved,
            scope: scope.clone(),
        }
    }

    fn filter_key(&self) -> EdgeFilterKey {
        EdgeFilterKey {
            kind: self.kind,
            include_unresolved: self.include_unresolved,
            scope: self.scope.clone(),
        }
    }

    fn from_filter_key(artefact_id: String, filter_key: &EdgeFilterKey) -> Self {
        Self {
            artefact_id,
            kind: filter_key.kind,
            include_unresolved: filter_key.include_unresolved,
            scope: filter_key.scope.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeFilterKey {
    kind: Option<EdgeKind>,
    include_unresolved: bool,
    scope: ResolverScope,
}

impl EdgeFilterKey {
    fn as_filter(&self, direction: DepsDirection) -> DepsFilterInput {
        DepsFilterInput {
            kind: self.kind,
            direction,
            include_unresolved: self.include_unresolved,
        }
    }
}
