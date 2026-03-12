use super::*;
use std::path::PathBuf;

struct BddScenarioCoverage {
    id: &'static str,
    covered_by: &'static str,
}

const BDD_MATRIX: &[BddScenarioCoverage] = &[
    BddScenarioCoverage {
        id: "S1",
        covered_by: "bdd_ts_js_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "S2",
        covered_by: "bdd_rust_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "S3",
        covered_by: "bdd_rust_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "S4",
        covered_by: "bdd_query_semantics_cover_reverse_and_bidirectional_deps",
    },
    BddScenarioCoverage {
        id: "S5",
        covered_by: "bdd_query_semantics_cover_reverse_and_bidirectional_deps",
    },
    BddScenarioCoverage {
        id: "E1",
        covered_by: "bdd_rust_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "E2",
        covered_by: "bdd_ts_js_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "E3",
        covered_by: "bdd_ts_js_fixture_covers_primary_artefacts_and_edges",
    },
    BddScenarioCoverage {
        id: "ERR1",
        covered_by: "bdd_parse_failure_fixture_degrades_to_file_only",
    },
    BddScenarioCoverage {
        id: "ERR2",
        covered_by: "bdd_query_semantics_cover_reverse_and_bidirectional_deps",
    },
];

#[test]
fn bdd_matrix_maps_all_bdd_scenarios() {
    let ids = BDD_MATRIX.iter().map(|entry| entry.id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "S1", "S2", "S3", "S4", "S5", "E1", "E2", "E3", "ERR1", "ERR2"
        ]
    );
    assert!(BDD_MATRIX.iter().all(|entry| !entry.covered_by.is_empty()));
}

#[test]
fn bdd_ts_js_fixture_covers_primary_artefacts_and_edges() {
    let content = r#"import defaultHelper, { helper } from "./helpers";
export { helper };
export { helper };
export { helper as helperAlias };

interface User {
  id: string;
}

type UserId = string;

function normalizeId(id: UserId): UserId {
  return id;
}

class BaseService {}

class Service extends BaseService {
  constructor(private readonly prefix: string) {}

  run(user: User): UserId {
    normalizeId(user.id);
    defaultHelper(user.id);
    missing();
    return helper(user.id);
  }
}
"#;

    let artefacts = extract_js_ts_artefacts(content, "src/sample.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/sample.ts", &artefacts).unwrap();

    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "import_statement" && artefact.canonical_kind == "import"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "interface_declaration"
            && artefact.canonical_kind == "interface"
            && artefact.name == "User"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "type_alias_declaration"
            && artefact.canonical_kind == "type"
            && artefact.name == "UserId"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "class_declaration"
            && artefact.canonical_kind == "language_only"
            && artefact.name == "Service"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "constructor"
            && artefact.canonical_kind == "language_only"
            && artefact.name == "constructor"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "method_definition"
            && artefact.canonical_kind == "method"
            && artefact.name == "run"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "function_declaration"
            && artefact.canonical_kind == "function"
            && artefact.name == "normalizeId"
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "imports" && edge.to_symbol_ref.as_deref() == Some("./helpers")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.from_symbol_fqn == "src/sample.ts::Service::run"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::normalizeId")
            && edge
                .metadata
                .get("resolution")
                .and_then(|value| value.as_str())
                == Some("local")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.from_symbol_fqn == "src/sample.ts::Service::run"
            && edge.to_symbol_ref.as_deref() == Some("./helpers::default")
            && edge
                .metadata
                .get("resolution")
                .and_then(|value| value.as_str())
                == Some("import")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "references"
            && edge.from_symbol_fqn == "src/sample.ts::Service::run"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::User")
            && edge
                .metadata
                .get("ref_kind")
                .and_then(|value| value.as_str())
                == Some("type")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/sample.ts::Service"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/sample.ts::BaseService")
    }));
    assert!(
        edges
            .iter()
            .filter(|edge| {
                edge.edge_kind == "exports"
                    && edge
                        .metadata
                        .get("export_name")
                        .and_then(|value| value.as_str())
                        == Some("helper")
            })
            .count()
            == 1
    );
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "exports"
            && edge
                .metadata
                .get("export_name")
                .and_then(|value| value.as_str())
                == Some("helperAlias")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.to_symbol_ref.as_deref() == Some("src/sample.ts::missing")
            && edge
                .metadata
                .get("resolution")
                .and_then(|value| value.as_str())
                == Some("unresolved")
    }));
}

#[test]
fn bdd_rust_fixture_covers_primary_artefacts_and_edges() {
    let content = r#"use crate::math::sum;

trait Reader {}
trait Writer {}

trait Repository: Reader + Writer {
    fn load(&self);
}

struct PgRepository;

impl Repository for PgRepository {
    fn load(&self) {
        helper();
        sum();
        println!("hi");
    }
}

pub fn helper() {}
pub use self::helper;
"#;

    let artefacts = extract_rust_artefacts(content, "src/lib.rs").unwrap();
    let edges = extract_rust_dependency_edges(content, "src/lib.rs", &artefacts).unwrap();

    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "use_declaration" && artefact.canonical_kind == "import"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "struct_item"
            && artefact.canonical_kind == "language_only"
            && artefact.name == "PgRepository"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "trait_item"
            && artefact.canonical_kind == "interface"
            && artefact.name == "Repository"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "impl_item" && artefact.canonical_kind == "language_only"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "function_item"
            && artefact.canonical_kind == "function"
            && artefact.name == "helper"
    }));
    assert!(artefacts.iter().any(|artefact| {
        artefact.language_kind == "function_item"
            && artefact.canonical_kind == "method"
            && artefact.name == "load"
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "imports" && edge.to_symbol_ref.as_deref() == Some("crate::math::sum")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "implements" && edge.to_symbol_ref.as_deref() == Some("Repository")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.from_symbol_fqn.ends_with("::load")
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
            && edge
                .metadata
                .get("resolution")
                .and_then(|value| value.as_str())
                == Some("local")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Reader")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "inherits"
            && edge.from_symbol_fqn == "src/lib.rs::Repository"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::Writer")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "calls"
            && edge.from_symbol_fqn.ends_with("::load")
            && edge
                .metadata
                .get("call_form")
                .and_then(|value| value.as_str())
                == Some("macro")
    }));
    assert!(edges.iter().any(|edge| {
        edge.edge_kind == "exports"
            && edge.to_target_symbol_fqn.as_deref() == Some("src/lib.rs::helper")
            && edge
                .metadata
                .get("export_form")
                .and_then(|value| value.as_str())
                == Some("pub_use")
    }));
}

#[test]
fn bdd_query_semantics_cover_reverse_and_bidirectional_deps() {
    let cfg = DevqlConfig {
        repo_root: PathBuf::from("/tmp/repo"),
        repo: RepoIdentity {
            provider: "github".to_string(),
            organization: "bitloops".to_string(),
            name: "temp2".to_string(),
            identity: "github/bitloops/temp2".to_string(),
            repo_id: deterministic_uuid("repo://github/bitloops/temp2"),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    };

    let inbound = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls",direction:"in",include_unresolved:false)->limit(10)"#,
    )
    .unwrap();
    let both = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"exports",direction:"both")->limit(10)"#,
    )
    .unwrap();
    let invalid = parse_devql_query(
        r#"repo("temp2")->artefacts(kind:"function")->deps(kind:"calls")->chatHistory()->limit(10)"#,
    )
    .unwrap();

    let inbound_sql = build_postgres_deps_query(&cfg, &inbound, &cfg.repo.repo_id).unwrap();
    let both_sql = build_postgres_deps_query(&cfg, &both, &cfg.repo.repo_id).unwrap();

    assert!(inbound_sql.contains("JOIN artefacts at ON at.artefact_id = e.to_artefact_id"));
    assert!(inbound_sql.contains("e.to_artefact_id IS NOT NULL"));
    assert!(both_sql.contains("WITH out_edges AS"));
    assert!(both_sql.contains("UNION ALL"));

    let err = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(execute_devql_query(&cfg, &invalid, None))
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("deps() cannot be combined with chatHistory()")
    );
}

#[test]
fn bdd_parse_failure_fixture_degrades_to_file_only() {
    let content = r#"function broken( {"#;
    let artefacts = extract_js_ts_artefacts(content, "src/broken.ts").unwrap();
    let edges = extract_js_ts_dependency_edges(content, "src/broken.ts", &artefacts).unwrap();

    assert!(artefacts.is_empty());
    assert!(edges.is_empty());
}
