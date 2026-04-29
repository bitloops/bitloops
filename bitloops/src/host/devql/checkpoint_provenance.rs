#![allow(dead_code)]

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CheckpointFileChangeKind {
    Add,
    Modify,
    Delete,
    Rename,
    Copy,
}

impl CheckpointFileChangeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Modify => "modify",
            Self::Delete => "delete",
            Self::Rename => "rename",
            Self::Copy => "copy",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "add" => Some(Self::Add),
            "modify" => Some(Self::Modify),
            "delete" => Some(Self::Delete),
            "rename" => Some(Self::Rename),
            "copy" => Some(Self::Copy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CheckpointArtefactChangeKind {
    Add,
    Modify,
    Delete,
}

impl CheckpointArtefactChangeKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Modify => "modify",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CheckpointProvenanceContext<'a> {
    pub repo_id: &'a str,
    pub checkpoint_id: &'a str,
    pub session_id: &'a str,
    pub event_time: &'a str,
    pub agent: &'a str,
    pub branch: &'a str,
    pub strategy: &'a str,
    pub commit_sha: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointFileProvenanceRow {
    pub relation_id: String,
    pub repo_id: String,
    pub checkpoint_id: String,
    pub session_id: String,
    pub event_time: String,
    pub agent: String,
    pub branch: String,
    pub strategy: String,
    pub commit_sha: String,
    pub change_kind: CheckpointFileChangeKind,
    pub path_before: Option<String>,
    pub path_after: Option<String>,
    pub blob_sha_before: Option<String>,
    pub blob_sha_after: Option<String>,
    pub copy_source_path: Option<String>,
    pub copy_source_blob_sha: Option<String>,
}

impl CheckpointFileProvenanceRow {
    pub(crate) fn display_path(&self) -> String {
        checkpoint_display_path(self.path_before.as_deref(), self.path_after.as_deref())
    }

    fn deterministic_id(&self) -> String {
        deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.repo_id,
            self.checkpoint_id,
            self.change_kind.as_str(),
            self.path_before.as_deref().unwrap_or(""),
            self.path_after.as_deref().unwrap_or(""),
            self.blob_sha_before.as_deref().unwrap_or(""),
            self.blob_sha_after.as_deref().unwrap_or(""),
            self.copy_source_path.as_deref().unwrap_or(""),
            self.copy_source_blob_sha.as_deref().unwrap_or(""),
            self.session_id,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointArtefactProvenanceRow {
    pub relation_id: String,
    pub repo_id: String,
    pub checkpoint_id: String,
    pub session_id: String,
    pub event_time: String,
    pub agent: String,
    pub branch: String,
    pub strategy: String,
    pub commit_sha: String,
    pub change_kind: CheckpointArtefactChangeKind,
    pub before_symbol_id: Option<String>,
    pub after_symbol_id: Option<String>,
    pub before_artefact_id: Option<String>,
    pub after_artefact_id: Option<String>,
}

impl CheckpointArtefactProvenanceRow {
    fn deterministic_id(&self) -> String {
        deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.repo_id,
            self.checkpoint_id,
            self.session_id,
            self.change_kind.as_str(),
            self.before_symbol_id.as_deref().unwrap_or(""),
            self.after_symbol_id.as_deref().unwrap_or(""),
            self.before_artefact_id.as_deref().unwrap_or(""),
            self.after_artefact_id.as_deref().unwrap_or(""),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CheckpointArtefactLineageKind {
    Copy,
}

impl CheckpointArtefactLineageKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointArtefactLineageRow {
    pub relation_id: String,
    pub repo_id: String,
    pub checkpoint_id: String,
    pub session_id: String,
    pub event_time: String,
    pub agent: String,
    pub branch: String,
    pub strategy: String,
    pub commit_sha: String,
    pub lineage_kind: CheckpointArtefactLineageKind,
    pub source_symbol_id: String,
    pub source_artefact_id: String,
    pub dest_symbol_id: String,
    pub dest_artefact_id: String,
}

impl CheckpointArtefactLineageRow {
    fn deterministic_id(&self) -> String {
        deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.repo_id,
            self.checkpoint_id,
            self.session_id,
            self.lineage_kind.as_str(),
            self.source_symbol_id,
            self.source_artefact_id,
            self.dest_symbol_id,
            self.dest_artefact_id,
        ))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CheckpointArtefactProvenanceBundle {
    pub semantic_rows: Vec<CheckpointArtefactProvenanceRow>,
    pub lineage_rows: Vec<CheckpointArtefactLineageRow>,
}

#[path = "checkpoint_provenance/artefacts.rs"]
mod artefacts;
#[path = "checkpoint_provenance/git.rs"]
mod git;
#[path = "checkpoint_provenance/query.rs"]
mod query;
#[path = "checkpoint_provenance/sql.rs"]
mod sql;

pub(crate) use self::artefacts::collect_checkpoint_artefact_provenance;
#[cfg(test)]
pub(crate) use self::artefacts::normalise_semantic_source;
pub(crate) use self::git::collect_checkpoint_file_provenance_rows;
#[allow(unused_imports)]
pub(crate) use self::query::{
    CheckpointArtefactMatch, CheckpointFileActivityFilter, CheckpointFileDebugRow,
    CheckpointFileExistsSql, CheckpointFileGateway, CheckpointFileProvenanceDetailRow,
    CheckpointFileScope, CheckpointFileSnapshotMatch, CheckpointSelectionEvidenceKind,
    CheckpointSelectionMatch, CheckpointSelectionMatchStrength, build_checkpoint_file_debug_sql,
    build_checkpoint_file_exists_clause, build_checkpoint_file_lookup_sql, checkpoint_display_path,
};
pub(crate) use self::sql::{
    build_upsert_checkpoint_artefact_lineage_row_sql, build_upsert_checkpoint_artefact_row_sql,
    build_upsert_checkpoint_file_row_sql, delete_checkpoint_artefact_lineage_rows_sql,
    delete_checkpoint_artefact_rows_sql, delete_checkpoint_file_rows_sql,
};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const PROVENANCE_TABLE_SQL: &str = "
        CREATE TABLE checkpoint_files (
            relation_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            checkpoint_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_time TEXT NOT NULL,
            agent TEXT NOT NULL,
            branch TEXT NOT NULL,
            strategy TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            path_before TEXT,
            path_after TEXT,
            blob_sha_before TEXT,
            blob_sha_after TEXT,
            copy_source_path TEXT,
            copy_source_blob_sha TEXT
        );

        CREATE TABLE checkpoint_artefact_lineage (
            relation_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            checkpoint_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_time TEXT NOT NULL,
            agent TEXT NOT NULL,
            branch TEXT NOT NULL,
            strategy TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            lineage_kind TEXT NOT NULL,
            source_symbol_id TEXT NOT NULL,
            source_artefact_id TEXT NOT NULL,
            dest_symbol_id TEXT NOT NULL,
            dest_artefact_id TEXT NOT NULL
        );

        CREATE TABLE checkpoint_artefacts (
            relation_id TEXT PRIMARY KEY,
            repo_id TEXT NOT NULL,
            checkpoint_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_time TEXT NOT NULL,
            agent TEXT NOT NULL,
            branch TEXT NOT NULL,
            strategy TEXT NOT NULL,
            commit_sha TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            before_symbol_id TEXT,
            after_symbol_id TEXT,
            before_artefact_id TEXT,
            after_artefact_id TEXT
        );
    ";

    async fn sqlite_relational_with_provenance(seed_sql: &str) -> RelationalStorage {
        let temp = tempdir().expect("temp dir");
        let db_path = temp.path().join("projection.sqlite");
        let sql = format!("{PROVENANCE_TABLE_SQL}{seed_sql}");
        sqlite_exec_path_allow_create(&db_path, &sql)
            .await
            .expect("seed checkpoint provenance");
        std::mem::forget(temp);
        RelationalStorage::local_only(db_path)
    }

    #[test]
    fn build_exists_clause_matches_after_snapshot_identity() {
        let sql = build_checkpoint_file_exists_clause(CheckpointFileExistsSql {
            repo_id: "repo-1",
            path_column: "a.path",
            blob_sha_column: "a.blob_sha",
            activity_filter: CheckpointFileActivityFilter {
                agent: Some("codex"),
                since: Some("2026-03-20T00:00:00Z"),
            },
        });

        assert!(sql.contains("FROM checkpoint_files cf"));
        assert!(sql.contains("cf.path_after = a.path"));
        assert!(sql.contains("cf.blob_sha_after = a.blob_sha"));
    }

    #[tokio::test]
    async fn list_matching_checkpoints_uses_after_snapshot_identity() {
        let relational = sqlite_relational_with_provenance(
            "
            INSERT INTO checkpoint_files VALUES
                ('row-1', 'repo-1', 'checkpoint-1', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-old', 'blob-1', NULL, NULL),
                ('row-2', 'repo-1', 'checkpoint-2', 'session-2', '2026-03-22T10:00:00Z', 'codex', 'main', 'manual', 'commit-2', 'rename', 'src/old.rs', 'src/lib.rs', 'blob-old', 'blob-1', NULL, NULL),
                ('row-3', 'repo-1', 'checkpoint-3', 'session-3', '2026-03-23T10:00:00Z', 'codex', 'main', 'manual', 'commit-3', 'delete', 'src/lib.rs', NULL, 'blob-1', NULL, NULL, NULL),
                ('row-4', 'repo-1', 'checkpoint-4', 'session-4', '2026-03-24T10:00:00Z', 'codex', 'main', 'manual', 'commit-4', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-1', 'blob-2', NULL, NULL);
            ",
        )
        .await;
        let gateway = CheckpointFileGateway::new(&relational);

        let rows = gateway
            .list_matching_checkpoints(
                "repo-1",
                "./src/lib.rs",
                "blob-1",
                CheckpointFileActivityFilter::default(),
                10,
            )
            .await
            .expect("lookup matching checkpoints");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].checkpoint_id, "checkpoint-2");
        assert_eq!(rows[1].checkpoint_id, "checkpoint-1");
    }

    #[tokio::test]
    async fn list_selection_checkpoint_ids_includes_file_relation_matches() {
        let relational = sqlite_relational_with_provenance(
            "
            INSERT INTO checkpoint_files VALUES
                ('file-row-1', 'repo-1', 'checkpoint-file', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-old', 'blob-new', NULL, NULL);
            ",
        )
        .await;
        let gateway = CheckpointFileGateway::new(&relational);

        let rows = gateway
            .list_checkpoint_ids_for_selection(
                "repo-1",
                &["missing-symbol".to_string()],
                &["./src/lib.rs".to_string()],
                CheckpointFileActivityFilter::default(),
            )
            .await
            .expect("lookup selection checkpoints");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].checkpoint_id, "checkpoint-file");
    }

    #[tokio::test]
    async fn list_selection_matches_reports_strongest_evidence_per_checkpoint() {
        let relational = sqlite_relational_with_provenance(
            "
            INSERT INTO checkpoint_files VALUES
                ('file-row-1', 'repo-1', 'checkpoint-1', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'modify', 'src/lib.rs', 'src/lib.rs', 'blob-old', 'blob-new', NULL, NULL);
            INSERT INTO checkpoint_artefacts VALUES
                ('artefact-row-1', 'repo-1', 'checkpoint-1', 'session-1', '2026-03-20T10:00:00Z', 'codex', 'main', 'manual', 'commit-1', 'modify', NULL, 'symbol-1', NULL, 'artefact-1');
            ",
        )
        .await;
        let gateway = CheckpointFileGateway::new(&relational);

        let rows = gateway
            .list_checkpoint_selection_matches(
                "repo-1",
                &["symbol-1".to_string()],
                &["./src/lib.rs".to_string()],
                CheckpointFileActivityFilter::default(),
            )
            .await
            .expect("lookup selection matches");

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.checkpoint_id, "checkpoint-1");
        assert_eq!(
            row.evidence_kind,
            CheckpointSelectionEvidenceKind::SymbolProvenance
        );
        assert_eq!(
            row.evidence_kinds,
            vec![
                CheckpointSelectionEvidenceKind::SymbolProvenance,
                CheckpointSelectionEvidenceKind::FileRelation,
            ]
        );
        assert_eq!(row.match_strength, CheckpointSelectionMatchStrength::High);
    }

    #[test]
    fn checkpoint_selection_evidence_orders_line_overlap_before_symbol_and_file() {
        let mut evidence = vec![
            CheckpointSelectionEvidenceKind::FileRelation,
            CheckpointSelectionEvidenceKind::LineOverlap,
            CheckpointSelectionEvidenceKind::SymbolProvenance,
        ];
        evidence.sort_by_key(|kind| std::cmp::Reverse(kind.as_rank()));

        assert_eq!(
            evidence,
            vec![
                CheckpointSelectionEvidenceKind::LineOverlap,
                CheckpointSelectionEvidenceKind::SymbolProvenance,
                CheckpointSelectionEvidenceKind::FileRelation,
            ]
        );
        assert_eq!(
            CheckpointSelectionMatchStrength::from_evidence_kind(
                CheckpointSelectionEvidenceKind::LineOverlap,
            ),
            CheckpointSelectionMatchStrength::High,
        );
        assert_eq!(
            CheckpointSelectionMatchStrength::from_evidence_kind(
                CheckpointSelectionEvidenceKind::FileRelation,
            ),
            CheckpointSelectionMatchStrength::Medium,
        );
    }

    #[test]
    fn normalise_semantic_source_removes_comment_only_noise() {
        let before = "fn demo() {\n    // hello\n    value();\n}\n";
        let after = "fn demo() {\n    value(); // world\n}\n";
        assert_eq!(
            normalise_semantic_source(before),
            normalise_semantic_source(after),
        );
    }
}
