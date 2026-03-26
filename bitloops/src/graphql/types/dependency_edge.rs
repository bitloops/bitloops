use async_graphql::{ComplexObject, Context, Enum, ID, InputObject, Result, SimpleObject};

use crate::graphql::{backend_error, loaders::DataLoaders};

use super::{Artefact, JsonScalar};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Enum)]
pub enum EdgeKind {
    Imports,
    Calls,
    References,
    Extends,
    Implements,
    Exports,
}

impl EdgeKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::References => "references",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::Exports => "exports",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum, Default)]
pub enum DepsDirection {
    #[default]
    Out,
    In,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, InputObject)]
pub struct DepsFilterInput {
    pub kind: Option<EdgeKind>,
    #[graphql(default)]
    pub direction: DepsDirection,
    #[graphql(default)]
    pub include_unresolved: bool,
}

impl Default for DepsFilterInput {
    fn default() -> Self {
        Self {
            kind: None,
            direction: DepsDirection::Out,
            include_unresolved: false,
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct DependencyEdge {
    pub id: ID,
    pub edge_kind: EdgeKind,
    pub language: String,
    pub from_artefact_id: ID,
    pub to_artefact_id: Option<ID>,
    pub to_symbol_ref: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub metadata: Option<JsonScalar>,
}

impl DependencyEdge {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}

#[ComplexObject]
impl DependencyEdge {
    #[graphql(name = "fromArtefact")]
    async fn source_artefact(&self, ctx: &Context<'_>) -> Result<Artefact> {
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.from_artefact_id.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve source artefact {}: {err:#}",
                    self.from_artefact_id.as_ref()
                ))
            })?
            .ok_or_else(|| {
                backend_error(format!(
                    "source artefact {} no longer exists",
                    self.from_artefact_id.as_ref()
                ))
            })
    }

    #[graphql(name = "toArtefact")]
    async fn target_artefact(&self, ctx: &Context<'_>) -> Result<Option<Artefact>> {
        let Some(to_artefact_id) = self.to_artefact_id.as_ref() else {
            return Ok(None);
        };

        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(to_artefact_id.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve target artefact {}: {err:#}",
                    to_artefact_id.as_ref()
                ))
            })
    }
}
