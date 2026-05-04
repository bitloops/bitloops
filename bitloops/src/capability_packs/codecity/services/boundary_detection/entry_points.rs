use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;

use crate::capability_packs::codecity::services::source_graph::{
    CodeCitySourceArtefact, CodeCitySourceGraph,
};

pub(super) fn detect_entry_points(
    files: &[String],
    artefacts: &[CodeCitySourceArtefact],
) -> Vec<String> {
    let mut entries = BTreeSet::new();
    for path in files {
        let basename = Path::new(path)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("");
        if matches!(
            basename,
            "main.go" | "main.py" | "index.ts" | "index.js" | "app.py" | "App.java" | "Program.cs"
        ) || path.ends_with("src/main.rs")
            || path.contains("/bin/")
            || path.starts_with("bin/")
            || (path.contains("/cmd/") && path.ends_with("/main.go"))
        {
            entries.insert(path.clone());
        }
    }

    for artefact in artefacts {
        let is_main = artefact
            .symbol_fqn
            .as_deref()
            .is_some_and(|symbol| symbol.ends_with("::main"))
            || artefact.symbol_id.eq_ignore_ascii_case("main")
            || artefact
                .signature
                .as_deref()
                .is_some_and(|signature| signature.contains(" main("));
        if is_main && files.contains(&artefact.path) {
            entries.insert(artefact.path.clone());
        }
    }

    entries.into_iter().collect()
}

pub(super) fn infer_entry_kind(path: &str) -> String {
    let basename = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    match basename {
        "index.ts" | "index.js" => "node_index".to_string(),
        "main.go" | "main.py" => "main".to_string(),
        _ => "entry_point".to_string(),
    }
}

pub(super) fn dependency_closure(
    entry: &str,
    files: &[String],
    source: &CodeCitySourceGraph,
) -> BTreeSet<String> {
    let file_set = files.iter().cloned().collect::<BTreeSet<_>>();
    let adjacency = source
        .edges
        .iter()
        .filter(|edge| file_set.contains(&edge.from_path) && file_set.contains(&edge.to_path))
        .fold(BTreeMap::<String, Vec<String>>::new(), |mut map, edge| {
            map.entry(edge.from_path.clone())
                .or_default()
                .push(edge.to_path.clone());
            map
        });

    let mut closure = BTreeSet::from([entry.to_string()]);
    let mut stack = vec![entry.to_string()];
    while let Some(path) = stack.pop() {
        if let Some(targets) = adjacency.get(&path) {
            for target in targets {
                if closure.insert(target.clone()) {
                    stack.push(target.clone());
                }
            }
        }
    }
    closure
}

pub(super) fn closure_overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let denominator = left.len().min(right.len());
    if denominator == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / denominator as f64
}
