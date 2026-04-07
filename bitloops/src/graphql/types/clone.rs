use async_graphql::{ComplexObject, Context, ID, InputObject, Result, SimpleObject};

use crate::capability_packs::semantic_clones::scoring::{
    CloneScoringOptions, MAX_ANN_NEIGHBORS, MIN_ANN_NEIGHBORS,
};
use crate::graphql::{ResolverScope, backend_error, bad_user_input_error, loaders::DataLoaders};

use super::{Artefact, JsonScalar};

#[derive(Debug, Clone, InputObject, Default)]
pub struct ClonesFilterInput {
    pub relation_kind: Option<String>,
    pub min_score: Option<f64>,
    pub neighbors: Option<i32>,
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

    pub(crate) fn neighbors_override(&self) -> Option<CloneScoringOptions> {
        self.neighbors
            .map(|value| value.clamp(MIN_ANN_NEIGHBORS as i32, MAX_ANN_NEIGHBORS as i32))
            .map(|value| CloneScoringOptions::new(value as usize))
    }

    pub(crate) fn validate_project_scope(&self) -> Result<()> {
        if self.neighbors_override().is_some() {
            return Err(bad_user_input_error(
                "`neighbors` override is only supported for artefact-scoped `clones` queries",
            ));
        }
        Ok(())
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

impl SemanticClone {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_override_is_clamped_to_scoring_bounds() {
        let low = ClonesFilterInput {
            neighbors: Some(0),
            ..Default::default()
        };
        assert_eq!(
            low.neighbors_override().map(|opts| opts.ann_neighbors),
            Some(MIN_ANN_NEIGHBORS)
        );

        let high = ClonesFilterInput {
            neighbors: Some(999),
            ..Default::default()
        };
        assert_eq!(
            high.neighbors_override().map(|opts| opts.ann_neighbors),
            Some(MAX_ANN_NEIGHBORS)
        );
    }

    #[test]
    fn project_scope_validation_rejects_neighbors_override() {
        let filter = ClonesFilterInput {
            neighbors: Some(5),
            ..Default::default()
        };
        let err = filter
            .validate_project_scope()
            .expect_err("must reject neighbors");
        assert!(format!("{err:?}").contains("only supported for artefact-scoped `clones` queries"));
    }
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
