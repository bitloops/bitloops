use super::*;

#[test]
fn parse_devql_pipeline_supports_project_stage_and_explicit_limit() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->project("packages/api")->artefacts()->limit(25)"#,
    )
    .expect("query parses");

    assert_eq!(parsed.project_path.as_deref(), Some("packages/api"));
    assert_eq!(parsed.limit, 25);
    assert!(parsed.has_limit_stage);
}

#[test]
fn compile_project_asof_artefacts_pipeline() {
    let parsed = parse_devql_query(
        r#"repo("monorepo")->project("packages/api")->asOf(ref:"main")->artefacts(kind:"function")->limit(50)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "monorepo") {
    project(path: "packages/api") {
      asOf(input: { ref: "main" }) {
        artefacts(filter: { kind: FUNCTION }, first: 50) {
          edges {
            node {
              id
              path
              symbolFqn
              canonicalKind
              languageKind
              startLine
              endLine
              language
            }
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_artefacts_with_clone_spans() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones()->limit(10)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    artefacts(filter: { kind: FUNCTION }) {
      edges {
        node {
          id
          path
          symbolFqn
          canonicalKind
          languageKind
          startLine
          endLine
          language
          clones(first: 10) {
            edges {
              node {
                id
                sourceArtefactId
                targetArtefactId
                sourceStartLine
                sourceEndLine
                targetStartLine
                targetEndLine
                relationKind
                score
              }
            }
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_clone_summary_stage_ignores_limit_and_targets_typed_field() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones(min_score:0.75)->summary()->limit(1)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    cloneSummary(filter: { kind: FUNCTION }, cloneFilter: { minScore: 0.75 }) {
      totalCount
      groups {
        relationKind
        count
      }
    }
  }
}"#
    );
}

#[test]
fn compile_file_clone_summary_stage_preserves_all_filters() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.rs")->artefacts(kind:"function",symbol_fqn:"src/main.rs::main",lines:1..20,agent:"codex",since:"2026-03-01")->clones(relation_kind:"similar_implementation",min_score:0.75)->summary()"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    file(path: "src/main.rs") {
      cloneSummary(filter: { kind: FUNCTION, symbolFqn: "src/main.rs::main", lines: { start: 1, end: 20 }, agent: "codex", since: "2026-03-01T00:00:00Z" }, cloneFilter: { relationKind: "similar_implementation", minScore: 0.75 }) {
        totalCount
        groups {
          relationKind
          count
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_clone_summary_stage_rejects_select_projection() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones()->summary()->select(total_count)"#,
    )
    .expect("query parses");

    let err = compile_devql_to_graphql(&parsed).expect_err("summary select() should fail");
    assert!(
        err.to_string()
            .contains("summary() does not support select() in the GraphQL compiler yet"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn compile_clone_summary_stage_rejects_invalid_since_literal() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(since:"not-a-date")->clones()->summary()"#,
    )
    .expect("query parses");

    let err = compile_devql_to_graphql(&parsed).expect_err("invalid datetime should fail");
    assert!(
        err.to_string()
            .contains("invalid datetime value `not-a-date`"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn compile_file_artefacts_with_chat_history_enrichment() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->file("src/main.rs")->artefacts(lines:1..20,kind:"function")->chatHistory()->limit(5)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    file(path: "src/main.rs") {
      artefacts(filter: { kind: FUNCTION, lines: { start: 1, end: 20 } }) {
        edges {
          node {
            id
            path
            symbolFqn
            canonicalKind
            languageKind
            startLine
            endLine
            language
            chatHistory(first: 5) {
              edges {
                node {
                  sessionId
                  agent
                  timestamp
                  role
                  content
                }
              }
            }
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_artefact_clones_pipeline_uses_user_facing_default_selection() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones(min_score:0.8)->limit(10)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    artefacts(filter: { kind: FUNCTION }) {
      edges {
        node {
          clones(filter: { minScore: 0.8 }, first: 10) {
            edges {
              node {
                relationKind
                score
                sourceArtefact {
                  path
                  symbolFqn
                }
                targetArtefact {
                  path
                  symbolFqn
                }
              }
            }
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_artefact_clones_pipeline_keeps_raw_mode_opt_in() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones(relation_kind:"similar_implementation",raw:true)->limit(10)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    artefacts(filter: { kind: FUNCTION }) {
      edges {
        node {
          clones(filter: { relationKind: "similar_implementation" }, first: 10) {
            edges {
              node {
                id
                sourceArtefactId
                targetArtefactId
                relationKind
                score
                metadata
              }
            }
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_project_deps_pipeline() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->project("packages/api")->deps(kind:"imports",direction:"out")->limit(100)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    project(path: "packages/api") {
      deps(filter: { kind: IMPORTS, direction: OUT, includeUnresolved: true }, first: 100) {
        edges {
          node {
            id
            edgeKind
            fromArtefactId
            toArtefactId
            toSymbolRef
            startLine
            endLine
          }
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_repository_knowledge_pipeline() {
    let parsed =
        parse_devql_query(r#"repo("bitloops-cli")->knowledge()->limit(10)"#).expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    knowledge(first: 10) {
      edges {
        node {
          id
          provider
          sourceKind
          canonicalExternalId
          externalUrl
          title
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_project_coverage_stage_with_filter() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->project("packages/api")->artefacts(kind:"function")->coverage()->limit(25)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    project(path: "packages/api") {
      coverage(filter: { kind: FUNCTION }, first: 25) {
        artefact {
          artefactId
          name
          kind
          filePath
          startLine
          endLine
        }
        coverage {
          coverageSource
          lineCoveragePct
          branchCoveragePct
          lineDataAvailable
          branchDataAvailable
          uncoveredLines
          branches {
            line
            block
            branch
            covered
            hitCount
          }
        }
        summary {
          uncoveredLineCount
          uncoveredBranchCount
          diagnosticCount
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_select_fields_to_graphql_names() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(agent:"claude-code")->select(path,canonical_kind,symbol_fqn,start_line,end_line)->limit(50)"#,
    )
    .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert_eq!(
        graphql,
        r#"query {
  repo(name: "bitloops-cli") {
    artefacts(filter: { agent: "claude-code" }, first: 50) {
      edges {
        node {
          path
          canonicalKind
          symbolFqn
          startLine
          endLine
        }
      }
    }
  }
}"#
    );
}

#[test]
fn compile_rejects_unknown_select_field() {
    let parsed = parse_devql_query(r#"repo("bitloops-cli")->artefacts()->select(unknown_field)"#)
        .expect("query parses");

    let err = compile_devql_to_graphql(&parsed).expect_err("unknown field must fail");

    assert!(
        err.to_string()
            .contains("unsupported select() field `unknown_field`"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_rejects_multiple_registered_stages() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->tests()->coverage()->limit(5)"#,
    )
    .expect("query parses");

    let err = compile_devql_to_graphql(&parsed).expect_err("multiple stages must fail");

    assert!(
        err.to_string()
            .contains("does not yet support multiple registered capability-pack stages"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_tests_stage_defaults_to_covering_test_line_range_fields() {
    let parsed =
        parse_devql_query(r#"repo("bitloops-cli")->artefacts(kind:"function")->tests()->limit(5)"#)
            .expect("query parses");

    let graphql = compile_devql_to_graphql(&parsed).expect("graphql compiles");

    assert!(
        graphql.contains("coveringTests"),
        "expected coveringTests selection in compiled graphql: {graphql}"
    );
    assert!(
        graphql.contains("startLine"),
        "expected covering test startLine in compiled graphql: {graphql}"
    );
    assert!(
        graphql.contains("endLine"),
        "expected covering test endLine in compiled graphql: {graphql}"
    );
    assert!(
        !graphql.contains("confidence"),
        "did not expect confidence in default tests() selections: {graphql}"
    );
    assert!(
        !graphql.contains("discoverySource"),
        "did not expect discoverySource in default tests() selections: {graphql}"
    );
    assert!(
        !graphql.contains("linkageSource"),
        "did not expect linkageSource in default tests() selections: {graphql}"
    );
    assert!(
        !graphql.contains("linkageStatus"),
        "did not expect linkageStatus in default tests() selections: {graphql}"
    );
}
