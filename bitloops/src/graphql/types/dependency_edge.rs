use async_graphql::{ComplexObject, Context, Enum, ID, InputObject, Result, SimpleObject};

use crate::graphql::{ResolverScope, backend_error, loaders::DataLoaders};

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
    #[graphql(default = true)]
    pub include_unresolved: bool,
}

impl Default for DepsFilterInput {
    fn default() -> Self {
        Self {
            kind: None,
            direction: DepsDirection::Out,
            include_unresolved: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DepsFilterInput;

    #[test]
    fn deps_filter_default_includes_unresolved_edges() {
        assert!(
            DepsFilterInput::default().include_unresolved,
            "default deps filter should include unresolved edges"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, InputObject, Default)]
pub struct DepsSummaryFilterInput {
    pub kind: Option<EdgeKind>,
    pub direction: Option<DepsDirection>,
    #[graphql(default)]
    pub unresolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, SimpleObject)]
pub struct DepsSummaryKindCounts {
    pub imports: i32,
    pub calls: i32,
    pub references: i32,
    pub extends: i32,
    pub implements: i32,
    pub exports: i32,
}

impl DepsSummaryKindCounts {
    pub(crate) fn increment(&mut self, kind: EdgeKind) {
        match kind {
            EdgeKind::Imports => self.imports = self.imports.saturating_add(1),
            EdgeKind::Calls => self.calls = self.calls.saturating_add(1),
            EdgeKind::References => self.references = self.references.saturating_add(1),
            EdgeKind::Extends => self.extends = self.extends.saturating_add(1),
            EdgeKind::Implements => self.implements = self.implements.saturating_add(1),
            EdgeKind::Exports => self.exports = self.exports.saturating_add(1),
        }
    }

    pub(crate) fn merge(&mut self, other: &Self) {
        self.imports = self.imports.saturating_add(other.imports);
        self.calls = self.calls.saturating_add(other.calls);
        self.references = self.references.saturating_add(other.references);
        self.extends = self.extends.saturating_add(other.extends);
        self.implements = self.implements.saturating_add(other.implements);
        self.exports = self.exports.saturating_add(other.exports);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, SimpleObject)]
pub struct DepsSummary {
    pub total_count: i32,
    pub incoming_count: i32,
    pub outgoing_count: i32,
    pub kind_counts: DepsSummaryKindCounts,
}

impl DepsSummary {
    pub(crate) fn from_edges(incoming: &[DependencyEdge], outgoing: &[DependencyEdge]) -> Self {
        fn count_kind(edges: &[DependencyEdge]) -> DepsSummaryKindCounts {
            let mut counts = DepsSummaryKindCounts::default();
            for edge in edges {
                counts.increment(edge.edge_kind);
            }
            counts
        }

        let incoming_count = i32::try_from(incoming.len()).unwrap_or(i32::MAX);
        let outgoing_count = i32::try_from(outgoing.len()).unwrap_or(i32::MAX);
        let total_count = incoming_count.saturating_add(outgoing_count);
        let incoming_kind_counts = count_kind(incoming);
        let outgoing_kind_counts = count_kind(outgoing);
        let mut kind_counts = incoming_kind_counts;
        kind_counts.merge(&outgoing_kind_counts);

        Self {
            total_count,
            incoming_count,
            outgoing_count,
            kind_counts,
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
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl DependencyEdge {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[ComplexObject]
impl DependencyEdge {
    #[graphql(name = "fromArtefact")]
    async fn source_artefact(&self, ctx: &Context<'_>) -> Result<Artefact> {
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.from_artefact_id.as_ref(), &self.scope)
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
            .load_artefact_by_id(to_artefact_id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve target artefact {}: {err:#}",
                    to_artefact_id.as_ref()
                ))
            })
    }
}
