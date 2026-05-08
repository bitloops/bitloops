mod resolvers;
mod stages;
mod support;

use async_graphql::{Enum, InputObject, SimpleObject};

use crate::graphql::{ResolverScope, bad_user_input_error};

use super::{Artefact, LineRangeInput};

pub use stages::{
    CheckpointStageResult, CloneExpandHint, CloneStageResult, DependencyExpandHint,
    DependencyStageResult, HistoricalContextItem, HistoricalContextStageResult,
    HistoricalEvidenceKind, HistoricalMatchReason, HistoricalMatchStrength, HistoricalToolEvent,
    TestsStageResult,
};
pub(crate) use support::captured_preview;
use support::{dedup_strings, saturating_i32};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtefactSelectorMode {
    SymbolFqn(String),
    Search {
        query: String,
        mode: SearchMode,
    },
    Path {
        path: String,
        lines: Option<LineRangeInput>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Enum)]
pub enum SearchMode {
    #[default]
    Auto,
    Identity,
    Code,
    Summary,
    Lexical,
}

#[derive(Debug, Clone, InputObject)]
pub struct ArtefactSelectorInput {
    pub symbol_fqn: Option<String>,
    pub search: Option<String>,
    pub search_mode: Option<SearchMode>,
    pub path: Option<String>,
    pub lines: Option<LineRangeInput>,
}

impl ArtefactSelectorInput {
    pub(crate) fn selection_mode(&self) -> async_graphql::Result<ArtefactSelectorMode> {
        let symbol_fqn = self
            .symbol_fqn
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let search = match self.search.as_deref() {
            Some(value) if value.trim().is_empty() => {
                return Err(bad_user_input_error(
                    "`selectArtefacts(by: ...)` requires a non-empty `search`",
                ));
            }
            Some(value) => Some(value.trim().to_string()),
            None => None,
        };
        let search_mode = self.search_mode.unwrap_or_default();
        let path = self
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let path_selector_requested = path.is_some() || self.lines.is_some();
        let selector_count = usize::from(symbol_fqn.is_some())
            + usize::from(search.is_some())
            + usize::from(path_selector_requested);
        if selector_count == 0 {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires exactly one selector mode",
            ));
        }
        if selector_count > 1 {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` allows exactly one of `symbolFqn`, `search`, or `path`/`lines`",
            ));
        }
        if path_selector_requested && path.is_none() {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` requires `path` when `lines` is provided",
            ));
        }
        if self.search_mode.is_some() && search.is_none() {
            return Err(bad_user_input_error(
                "`selectArtefacts(by: ...)` only allows `searchMode` when `search` is provided",
            ));
        }

        if let Some(symbol_fqn) = symbol_fqn {
            return Ok(ArtefactSelectorMode::SymbolFqn(symbol_fqn));
        }
        if let Some(search) = search {
            return Ok(ArtefactSelectorMode::Search {
                query: search,
                mode: search_mode,
            });
        }

        let path = path.expect("selector_count ensures path selector exists");
        if let Some(lines) = self.lines.as_ref() {
            lines.validate()?;
        }
        Ok(ArtefactSelectorMode::Path {
            path,
            lines: self.lines.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum DirectoryEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct DirectoryEntry {
    pub path: String,
    pub name: String,
    pub entry_kind: DirectoryEntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtefactSelectionMode {
    Artefacts,
    DirectoryEntries,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct SearchBreakdown {
    pub lexical: Vec<Artefact>,
    pub identity: Vec<Artefact>,
    pub code: Vec<Artefact>,
    pub summary: Vec<Artefact>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(complex)]
pub struct ArtefactSelection {
    pub count: IntCount,
    #[graphql(skip)]
    mode: ArtefactSelectionMode,
    #[graphql(skip)]
    pub(crate) artefacts: Vec<Artefact>,
    #[graphql(skip)]
    pub(crate) directory_entries: Vec<DirectoryEntry>,
    #[graphql(skip)]
    pub(crate) search_breakdown: Option<SearchBreakdown>,
    #[graphql(skip)]
    pub(crate) search_query: Option<String>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

pub type IntCount = i32;

impl ArtefactSelection {
    pub(crate) fn new(
        artefacts: Vec<Artefact>,
        directory_entries: Vec<DirectoryEntry>,
        scope: ResolverScope,
    ) -> Self {
        Self {
            count: saturating_i32(artefacts.len()),
            mode: ArtefactSelectionMode::Artefacts,
            artefacts,
            directory_entries,
            search_breakdown: None,
            search_query: None,
            scope,
        }
    }

    pub(crate) fn new_search(
        artefacts: Vec<Artefact>,
        search_breakdown: Option<SearchBreakdown>,
        search_query: String,
        scope: ResolverScope,
    ) -> Self {
        Self {
            count: saturating_i32(artefacts.len()),
            mode: ArtefactSelectionMode::Artefacts,
            artefacts,
            directory_entries: Vec::new(),
            search_breakdown,
            search_query: Some(search_query),
            scope,
        }
    }

    pub(crate) fn from_directory_entries(
        directory_entries: Vec<DirectoryEntry>,
        scope: ResolverScope,
    ) -> Self {
        Self {
            count: saturating_i32(directory_entries.len()),
            mode: ArtefactSelectionMode::DirectoryEntries,
            artefacts: Vec::new(),
            directory_entries,
            search_breakdown: None,
            search_query: None,
            scope,
        }
    }

    fn artefact_ids(&self) -> Vec<String> {
        dedup_strings(self.artefacts.iter().map(|artefact| artefact.id.as_ref()))
    }

    fn symbol_ids(&self) -> Vec<String> {
        dedup_strings(
            self.artefacts
                .iter()
                .map(|artefact| artefact.symbol_id.as_str()),
        )
    }

    fn paths(&self) -> Vec<String> {
        dedup_strings(self.artefacts.iter().map(|artefact| artefact.path.as_str()))
    }
}

#[cfg(test)]
#[path = "artefact_selection_tests.rs"]
mod tests;
