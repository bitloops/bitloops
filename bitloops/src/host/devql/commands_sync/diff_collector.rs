use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::host::capability_host::events::{
    ChangedArtefact, ChangedFile, RemovedArtefact, RemovedFile, SyncArtefactDiff, SyncFileDiff,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffArtefactRecord {
    pub(crate) path: String,
    pub(crate) artefact_id: String,
    pub(crate) symbol_id: String,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) name: String,
}

impl DiffArtefactRecord {
    pub(crate) fn new(
        path: impl Into<String>,
        artefact_id: impl Into<String>,
        symbol_id: impl Into<String>,
        canonical_kind: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            artefact_id: artefact_id.into(),
            symbol_id: symbol_id.into(),
            canonical_kind,
            name: name.into(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SyncDiffCollector {
    file_diff: SyncFileDiff,
    pre_artefacts: HashMap<String, Vec<DiffArtefactRecord>>,
    post_artefacts: HashMap<String, Vec<DiffArtefactRecord>>,
}

impl SyncDiffCollector {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_file_added(&mut self, path: String, language: String, content_id: String) {
        self.file_diff.added.push(ChangedFile {
            path,
            language,
            content_id,
        });
    }

    pub(crate) fn record_file_changed(
        &mut self,
        path: String,
        language: String,
        content_id: String,
    ) {
        self.file_diff.changed.push(ChangedFile {
            path,
            language,
            content_id,
        });
    }

    pub(crate) fn record_file_removed(&mut self, path: String) {
        self.file_diff.removed.push(RemovedFile { path });
    }

    pub(crate) fn record_pre_artefacts(
        &mut self,
        path: impl Into<String>,
        artefacts: Vec<DiffArtefactRecord>,
    ) {
        self.pre_artefacts
            .entry(path.into())
            .or_default()
            .extend(artefacts);
    }

    pub(crate) fn record_post_artefacts(
        &mut self,
        path: impl Into<String>,
        artefacts: Vec<DiffArtefactRecord>,
    ) {
        self.post_artefacts
            .entry(path.into())
            .or_default()
            .extend(artefacts);
    }

    pub(crate) fn into_diffs(mut self) -> (SyncFileDiff, SyncArtefactDiff) {
        sort_file_diff(&mut self.file_diff);

        let mut artefact_diff = SyncArtefactDiff::default();
        let all_paths = self
            .pre_artefacts
            .keys()
            .chain(self.post_artefacts.keys())
            .cloned()
            .collect::<BTreeSet<_>>();

        for path in all_paths {
            let pre_by_symbol = by_symbol_id(self.pre_artefacts.remove(&path).unwrap_or_default());
            let post_by_symbol =
                by_symbol_id(self.post_artefacts.remove(&path).unwrap_or_default());

            for (symbol_id, post) in &post_by_symbol {
                match pre_by_symbol.get(symbol_id) {
                    Some(pre) if pre.artefact_id != post.artefact_id => {
                        artefact_diff.changed.push(ChangedArtefact {
                            artefact_id: post.artefact_id.clone(),
                            symbol_id: post.symbol_id.clone(),
                            path: post.path.clone(),
                            canonical_kind: post.canonical_kind.clone(),
                            name: post.name.clone(),
                        });
                    }
                    None => {
                        artefact_diff.added.push(ChangedArtefact {
                            artefact_id: post.artefact_id.clone(),
                            symbol_id: post.symbol_id.clone(),
                            path: post.path.clone(),
                            canonical_kind: post.canonical_kind.clone(),
                            name: post.name.clone(),
                        });
                    }
                    _ => {}
                }
            }

            for (symbol_id, pre) in pre_by_symbol {
                if !post_by_symbol.contains_key(&symbol_id) {
                    artefact_diff.removed.push(RemovedArtefact {
                        artefact_id: pre.artefact_id,
                        symbol_id: pre.symbol_id,
                        path: pre.path,
                    });
                }
            }
        }

        sort_artefact_diff(&mut artefact_diff);
        (self.file_diff, artefact_diff)
    }
}

fn by_symbol_id(artefacts: Vec<DiffArtefactRecord>) -> BTreeMap<String, DiffArtefactRecord> {
    let mut by_symbol = BTreeMap::new();
    for artefact in artefacts {
        by_symbol.insert(artefact.symbol_id.clone(), artefact);
    }
    by_symbol
}

fn sort_file_diff(diff: &mut SyncFileDiff) {
    diff.added.sort_by(|left, right| left.path.cmp(&right.path));
    diff.changed
        .sort_by(|left, right| left.path.cmp(&right.path));
    diff.removed
        .sort_by(|left, right| left.path.cmp(&right.path));
}

fn sort_artefact_diff(diff: &mut SyncArtefactDiff) {
    diff.added.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.symbol_id.cmp(&right.symbol_id))
    });
    diff.changed.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.symbol_id.cmp(&right.symbol_id))
    });
    diff.removed.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.symbol_id.cmp(&right.symbol_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_path_and_artefact_are_reported_as_added() {
        let mut collector = SyncDiffCollector::new();
        collector.record_file_added("src/a.rs".into(), "rust".into(), "blob-a".into());
        collector.record_post_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-1",
                "sid-1",
                Some("function".into()),
                "foo",
            )],
        );

        let (files, artefacts) = collector.into_diffs();
        assert_eq!(files.added.len(), 1);
        assert_eq!(files.added[0].path, "src/a.rs");
        assert_eq!(artefacts.added.len(), 1);
        assert_eq!(artefacts.added[0].symbol_id, "sid-1");
        assert!(artefacts.changed.is_empty());
        assert!(artefacts.removed.is_empty());
    }

    #[test]
    fn changed_artefact_id_with_same_symbol_is_reported_as_changed() {
        let mut collector = SyncDiffCollector::new();
        collector.record_file_changed("src/a.rs".into(), "rust".into(), "blob-new".into());
        collector.record_pre_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-old",
                "sid-1",
                Some("function".into()),
                "foo",
            )],
        );
        collector.record_post_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-new",
                "sid-1",
                Some("function".into()),
                "foo",
            )],
        );

        let (_files, artefacts) = collector.into_diffs();
        assert_eq!(artefacts.changed.len(), 1);
        assert_eq!(artefacts.changed[0].artefact_id, "aid-new");
        assert!(artefacts.added.is_empty());
        assert!(artefacts.removed.is_empty());
    }

    #[test]
    fn removed_path_is_reported_as_removed() {
        let mut collector = SyncDiffCollector::new();
        collector.record_file_removed("src/a.rs".into());
        collector.record_pre_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-1",
                "sid-1",
                Some("function".into()),
                "foo",
            )],
        );

        let (files, artefacts) = collector.into_diffs();
        assert_eq!(files.removed.len(), 1);
        assert_eq!(artefacts.removed.len(), 1);
        assert_eq!(artefacts.removed[0].symbol_id, "sid-1");
        assert!(artefacts.added.is_empty());
        assert!(artefacts.changed.is_empty());
    }

    #[test]
    fn renamed_symbol_is_reported_as_removed_and_added() {
        let mut collector = SyncDiffCollector::new();
        collector.record_file_changed("src/a.rs".into(), "rust".into(), "blob-new".into());
        collector.record_pre_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-old",
                "sid-old",
                Some("function".into()),
                "old_name",
            )],
        );
        collector.record_post_artefacts(
            "src/a.rs",
            vec![DiffArtefactRecord::new(
                "src/a.rs",
                "aid-new",
                "sid-new",
                Some("function".into()),
                "new_name",
            )],
        );

        let (_files, artefacts) = collector.into_diffs();
        assert_eq!(artefacts.added.len(), 1);
        assert_eq!(artefacts.added[0].name, "new_name");
        assert_eq!(artefacts.removed.len(), 1);
        assert_eq!(artefacts.removed[0].symbol_id, "sid-old");
        assert!(artefacts.changed.is_empty());
    }
}
