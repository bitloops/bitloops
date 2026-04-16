use async_graphql::{Result, SimpleObject};

use crate::graphql::{bad_cursor_error, bad_user_input_error};

use super::{
    Artefact, ChatEntry, Checkpoint, CloneSummary, Commit, DependencyEdge, InteractionEventObject,
    InteractionSessionObject, InteractionTurnObject, KnowledgeItem, KnowledgeRelation,
    KnowledgeVersion, SemanticClone, TelemetryEvent,
};

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

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct ChatEntryEdge {
    pub node: ChatEntry,
    pub cursor: String,
}

impl ChatEntryEdge {
    pub fn new(node: ChatEntry) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct ChatEntryConnection {
    pub edges: Vec<ChatEntryEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl ChatEntryConnection {
    pub fn new(edges: Vec<ChatEntryEdge>, page_info: PageInfo, total_count: usize) -> Self {
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

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct CloneEdge {
    pub node: SemanticClone,
    pub cursor: String,
}

impl CloneEdge {
    pub fn new(node: SemanticClone) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, PartialEq, SimpleObject)]
pub struct CloneConnection {
    pub edges: Vec<CloneEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
    pub summary: CloneSummary,
}

impl CloneConnection {
    pub fn new(
        edges: Vec<CloneEdge>,
        page_info: PageInfo,
        total_count: usize,
        summary: CloneSummary,
    ) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
            summary,
        }
    }
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
pub struct TelemetryEventEdge {
    pub node: TelemetryEvent,
    pub cursor: String,
}

impl TelemetryEventEdge {
    pub fn new(node: TelemetryEvent) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct TelemetryEventConnection {
    pub edges: Vec<TelemetryEventEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl TelemetryEventConnection {
    pub fn new(edges: Vec<TelemetryEventEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionSessionEdge {
    pub node: InteractionSessionObject,
    pub cursor: String,
}

impl InteractionSessionEdge {
    pub fn new(node: InteractionSessionObject) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionSessionConnection {
    pub edges: Vec<InteractionSessionEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl InteractionSessionConnection {
    pub fn new(
        edges: Vec<InteractionSessionEdge>,
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

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionTurnEdge {
    pub node: InteractionTurnObject,
    pub cursor: String,
}

impl InteractionTurnEdge {
    pub fn new(node: InteractionTurnObject) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionTurnConnection {
    pub edges: Vec<InteractionTurnEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl InteractionTurnConnection {
    pub fn new(edges: Vec<InteractionTurnEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionEventEdge {
    pub node: InteractionEventObject,
    pub cursor: String,
}

impl InteractionEventEdge {
    pub fn new(node: InteractionEventObject) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct InteractionEventConnection {
    pub edges: Vec<InteractionEventEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl InteractionEventConnection {
    pub fn new(edges: Vec<InteractionEventEdge>, page_info: PageInfo, total_count: usize) -> Self {
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

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeItemEdge {
    pub node: KnowledgeItem,
    pub cursor: String,
}

impl KnowledgeItemEdge {
    pub fn new(node: KnowledgeItem) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeItemConnection {
    pub edges: Vec<KnowledgeItemEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl KnowledgeItemConnection {
    pub fn new(edges: Vec<KnowledgeItemEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeVersionEdge {
    pub node: KnowledgeVersion,
    pub cursor: String,
}

impl KnowledgeVersionEdge {
    pub fn new(node: KnowledgeVersion) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeVersionConnection {
    pub edges: Vec<KnowledgeVersionEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl KnowledgeVersionConnection {
    pub fn new(edges: Vec<KnowledgeVersionEdge>, page_info: PageInfo, total_count: usize) -> Self {
        Self {
            edges,
            page_info,
            total_count: total_count.try_into().unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeRelationEdge {
    pub node: KnowledgeRelation,
    pub cursor: String,
}

impl KnowledgeRelationEdge {
    pub fn new(node: KnowledgeRelation) -> Self {
        let cursor = node.cursor();
        Self { node, cursor }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub struct KnowledgeRelationConnection {
    pub edges: Vec<KnowledgeRelationEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

impl KnowledgeRelationConnection {
    pub fn new(edges: Vec<KnowledgeRelationEdge>, page_info: PageInfo, total_count: usize) -> Self {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionPagination {
    Forward {
        limit: usize,
        after: Option<String>,
    },
    Backward {
        limit: usize,
        before: Option<String>,
    },
}

impl ConnectionPagination {
    pub fn from_graphql(
        default_limit: usize,
        first: Option<i32>,
        after: Option<&str>,
        last: Option<i32>,
        before: Option<&str>,
    ) -> Result<Self> {
        if (first.is_some() || after.is_some()) && (last.is_some() || before.is_some()) {
            return Err(bad_user_input_error(
                "use either forward pagination (`first`/`after`) or backward pagination (`last`/`before`), not both",
            ));
        }

        if let Some(last) = last {
            if last <= 0 {
                return Err(bad_user_input_error("`last` must be greater than zero"));
            }
            return Ok(Self::Backward {
                limit: last as usize,
                before: before.map(str::to_string),
            });
        }

        if before.is_some() {
            return Err(bad_user_input_error("`before` requires `last`"));
        }

        let first = first.unwrap_or(default_limit as i32);
        if first <= 0 {
            return Err(bad_user_input_error("`first` must be greater than zero"));
        }

        Ok(Self::Forward {
            limit: first as usize,
            after: after.map(str::to_string),
        })
    }

    pub fn limit(&self) -> usize {
        match self {
            Self::Forward { limit, .. } | Self::Backward { limit, .. } => *limit,
        }
    }

    pub fn fetch_limit(&self) -> usize {
        self.limit().saturating_add(1)
    }

    pub fn after(&self) -> Option<&str> {
        match self {
            Self::Forward { after, .. } => after.as_deref(),
            Self::Backward { .. } => None,
        }
    }

    pub fn before(&self) -> Option<&str> {
        match self {
            Self::Backward { before, .. } => before.as_deref(),
            Self::Forward { .. } => None,
        }
    }
}

pub fn paginate_items<T: Clone>(
    items: &[T],
    pagination: &ConnectionPagination,
    cursor_of: impl Fn(&T) -> String,
) -> Result<PaginatedItems<T>> {
    let total_count = items.len();
    let (start_index, end_index) = match pagination {
        ConnectionPagination::Forward { limit, after } => {
            let start_index = match after.as_deref() {
                Some(cursor) => {
                    let Some(position) = items.iter().position(|item| cursor_of(item) == cursor)
                    else {
                        return Err(bad_cursor_error(format!(
                            "cursor `{cursor}` does not match any result in this connection"
                        )));
                    };
                    position.saturating_add(1)
                }
                None => 0,
            };
            let end_index = start_index.saturating_add(*limit).min(total_count);
            (start_index, end_index)
        }
        ConnectionPagination::Backward { limit, before } => {
            let end_index = match before.as_deref() {
                Some(cursor) => {
                    let Some(position) = items.iter().position(|item| cursor_of(item) == cursor)
                    else {
                        return Err(bad_cursor_error(format!(
                            "cursor `{cursor}` does not match any result in this connection"
                        )));
                    };
                    position
                }
                None => total_count,
            };
            let start_index = end_index.saturating_sub(*limit);
            (start_index, end_index)
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pagination(
        default_limit: usize,
        first: Option<i32>,
        after: Option<&str>,
        last: Option<i32>,
        before: Option<&str>,
    ) -> ConnectionPagination {
        ConnectionPagination::from_graphql(default_limit, first, after, last, before)
            .expect("pagination args should be valid")
    }

    #[test]
    fn paginate_items_defaults_to_forward_window() {
        let items = vec!["a", "b", "c"];
        let page = paginate_items(&items, &pagination(2, None, None, None, None), |item| {
            item.to_string()
        })
        .expect("forward page");

        assert_eq!(page.items, vec!["a", "b"]);
        assert_eq!(
            page.page_info,
            PageInfo {
                has_next_page: true,
                has_previous_page: false,
                start_cursor: Some("a".to_string()),
                end_cursor: Some("b".to_string()),
            }
        );
    }

    #[test]
    fn paginate_items_supports_backward_windows() {
        let items = vec!["a", "b", "c", "d"];
        let page = paginate_items(
            &items,
            &pagination(10, None, None, Some(2), Some("d")),
            |item| item.to_string(),
        )
        .expect("backward page");

        assert_eq!(page.items, vec!["b", "c"]);
        assert_eq!(
            page.page_info,
            PageInfo {
                has_next_page: true,
                has_previous_page: true,
                start_cursor: Some("b".to_string()),
                end_cursor: Some("c".to_string()),
            }
        );
    }

    #[test]
    fn paginate_items_supports_backward_window_from_tail() {
        let items = vec!["a", "b", "c"];
        let page = paginate_items(&items, &pagination(10, None, None, Some(2), None), |item| {
            item.to_string()
        })
        .expect("backward tail page");

        assert_eq!(page.items, vec!["b", "c"]);
        assert_eq!(
            page.page_info,
            PageInfo {
                has_next_page: false,
                has_previous_page: true,
                start_cursor: Some("b".to_string()),
                end_cursor: Some("c".to_string()),
            }
        );
    }

    #[test]
    fn pagination_rejects_mixed_directions() {
        let err = ConnectionPagination::from_graphql(10, Some(2), None, Some(2), None)
            .expect_err("mixed pagination must fail");

        assert!(
            err.message
                .contains("use either forward pagination (`first`/`after`) or backward pagination (`last`/`before`)")
        );
    }

    #[test]
    fn pagination_rejects_before_without_last() {
        let err = ConnectionPagination::from_graphql(10, None, None, None, Some("cursor"))
            .expect_err("before without last must fail");

        assert!(err.message.contains("`before` requires `last`"));
    }
}
