use anyhow::Result as AnyResult;
use async_graphql::{ComplexObject, Context, ID, InputObject, Result, SimpleObject};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error, loaders::DataLoaders,
};

use super::{Artefact, ArtefactFilterInput, JsonScalar};

#[derive(Debug, Clone, InputObject, Default)]
pub struct ClonesFilterInput {
    pub relation_kind: Option<String>,
    pub min_score: Option<f64>,
}

impl ClonesFilterInput {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(min_score) = self.min_score
            && !(0.0..=1.0).contains(&min_score)
        {
            return Err(bad_user_input_error("`minScore` must be between 0 and 1"));
        }

        Ok(())
    }

    pub(crate) fn relation_kind(&self) -> Option<&str> {
        self.relation_kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
#[graphql(complex, name = "Clone")]
pub struct SemanticClone {
    pub id: ID,
    pub source_artefact_id: ID,
    pub target_artefact_id: ID,
    pub relation_kind: String,
    pub score: f64,
    pub metadata: Option<JsonScalar>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct CloneSummaryGroup {
    pub relation_kind: String,
    pub count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct CloneSummary {
    pub total_count: i32,
    pub groups: Vec<CloneSummaryGroup>,
}

impl CloneSummary {
    pub(crate) fn empty() -> Self {
        Self {
            total_count: 0,
            groups: Vec::new(),
        }
    }

    pub(crate) fn from_counts(counts: BTreeMap<String, usize>) -> Self {
        if counts.is_empty() {
            return Self::empty();
        }

        let total_count = counts
            .values()
            .copied()
            .sum::<usize>()
            .try_into()
            .unwrap_or(i32::MAX);
        let mut groups = counts
            .into_iter()
            .map(|(relation_kind, count)| CloneSummaryGroup {
                relation_kind,
                count: count.try_into().unwrap_or(i32::MAX),
            })
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.relation_kind.cmp(&right.relation_kind))
        });

        Self { total_count, groups }
    }
}

impl SemanticClone {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

pub(crate) async fn resolve_clone_summary(
    context: &DevqlGraphqlContext,
    path: Option<&str>,
    artefact_filter: Option<&ArtefactFilterInput>,
    clone_filter: Option<&ClonesFilterInput>,
    scope: &ResolverScope,
) -> AnyResult<CloneSummary> {
    let artefacts = context.list_artefacts(path, artefact_filter, scope).await?;
    if artefacts.is_empty() {
        return Ok(CloneSummary::empty());
    }

    let mut counts = BTreeMap::<String, usize>::new();
    let mut seen_clone_ids = HashSet::<String>::new();

    for artefact in artefacts {
        let clones = context
            .list_artefact_clones(artefact.id.as_ref(), clone_filter, scope)
            .await?;
        for clone in clones {
            if !seen_clone_ids.insert(clone.id.to_string()) {
                continue;
            }
            *counts.entry(clone.relation_kind).or_default() += 1;
        }
    }

    Ok(CloneSummary::from_counts(counts))
}

#[ComplexObject]
impl SemanticClone {
    async fn source_artefact(&self, ctx: &Context<'_>) -> Result<Artefact> {
        let lookup_scope = self.scope.without_project_path();
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.source_artefact_id.as_ref(), &lookup_scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve source artefact {} for clone {}: {err:#}",
                    self.source_artefact_id.as_ref(),
                    self.id.as_ref()
                ))
            })?
            .ok_or_else(|| {
                backend_error(format!(
                    "source artefact {} for clone {} was not found",
                    self.source_artefact_id.as_ref(),
                    self.id.as_ref()
                ))
            })
    }

    async fn target_artefact(&self, ctx: &Context<'_>) -> Result<Artefact> {
        let lookup_scope = self.scope.without_project_path();
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.target_artefact_id.as_ref(), &lookup_scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve target artefact {} for clone {}: {err:#}",
                    self.target_artefact_id.as_ref(),
                    self.id.as_ref()
                ))
            })?
            .ok_or_else(|| {
                backend_error(format!(
                    "target artefact {} for clone {} was not found",
                    self.target_artefact_id.as_ref(),
                    self.id.as_ref()
                ))
            })
    }
}
