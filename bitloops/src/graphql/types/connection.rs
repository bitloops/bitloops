use async_graphql::{Result, SimpleObject};

use crate::graphql::{bad_cursor_error, bad_user_input_error};

use super::{Artefact, Checkpoint, Commit, DependencyEdge};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct PageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct CommitEdge {
    pub node: Commit,
    pub cursor: String,
}

impl CommitEdge {
    pub fn new(node: Commit) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct CommitConnection {
    pub edges: Vec<CommitEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl CommitConnection {
    pub fn new(edges: Vec<CommitEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ArtefactEdge {
    pub node: Artefact,
    pub cursor: String,
}

impl ArtefactEdge {
    pub fn new(node: Artefact) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct ArtefactConnection {
    pub edges: Vec<ArtefactEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl ArtefactConnection {
    pub fn new(edges: Vec<ArtefactEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct CheckpointEdge {
    pub node: Checkpoint,
    pub cursor: String,
}

impl CheckpointEdge {
    pub fn new(node: Checkpoint) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct CheckpointConnection {
    pub edges: Vec<CheckpointEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl CheckpointConnection {
    pub fn new(edges: Vec<CheckpointEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct DependencyConnectionEdge {
    pub node: DependencyEdge,
    pub cursor: String,
}

impl DependencyConnectionEdge {
    pub fn new(node: DependencyEdge) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct DependencyEdgeConnection {
    pub edges: Vec<DependencyConnectionEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl DependencyEdgeConnection {
    pub fn new(
        edges: Vec<DependencyConnectionEdge>,
        page_info: PageInfo,
        total_count: usize,
    ) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaginatedItems<T> {
    pub items: Vec<T>,
    pub page_info: PageInfo,
    pub total_count: usize,
}

pub fn paginate_items<T: Clone>(
    items: &[T],
    first: i32,
    after: Option<&str>,
    cursor_of: impl Fn(&T) -> String,
) -> Result<PaginatedItems<T>> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than zero"));
    }

    let total_count = items.len();
    let start_index = match after {
        Some(cursor) => {
            let Some(position) = items.iter().position(|item| cursor_of(item) == cursor) else {
                return Err(bad_cursor_error(format!(
                    "cursor `{cursor}` does not match any result in this connection"
                )));
            };
            position.saturating_add(1)
        }
        None => 0,
    };

    let end_index = start_index.saturating_add(first as usize).min(total_count);
    let page_items = items[start_index..end_index].to_vec();
    let start_cursor = page_items.first().map(&cursor_of);
    let end_cursor = page_items.last().map(&cursor_of);

    Ok(PaginatedItems {
        items: page_items,
        page_info: PageInfo {
            has_next_page: end_index < total_count,
            has_previous_page: start_index > 0,
            start_cursor,
            end_cursor,
        },
        total_count,
    })
}
