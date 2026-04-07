use super::*;

#[test]
fn compile_query_document_compiles_devql_pipeline() {
    let document = compile_query_document(r#"repo("bitloops-cli")->artefacts()->limit(2)"#, false)
        .expect("dsl query should compile");

    assert!(document.contains("repo(name: \"bitloops-cli\")"));
    assert!(document.contains("artefacts(first: 2)"));
}

#[test]
fn compile_query_document_preserves_raw_graphql() {
    let document = compile_query_document(" { repo(name: \"bitloops-cli\") { name } } ", true)
        .expect("raw graphql should pass through");

    assert_eq!(document, "{ repo(name: \"bitloops-cli\") { name } }");
}

#[test]
fn compile_query_document_defaults_to_raw_graphql_without_pipeline_operator() {
    let document = compile_query_document("{ repo(name: \"bitloops-cli\") { name } }", false)
        .expect("graphql should be the default mode");

    assert_eq!(document, "{ repo(name: \"bitloops-cli\") { name } }");
}

#[test]
fn use_raw_graphql_mode_treats_pipeline_operator_as_dsl_only() {
    assert!(!use_raw_graphql_mode(
        r#"repo("bitloops-cli")->artefacts()->limit(2)"#,
        false
    ));
    assert!(use_raw_graphql_mode(
        r#"{ repo(name: "bitloops-cli") { name } }"#,
        false
    ));
    assert!(use_raw_graphql_mode(
        r#"repo("bitloops-cli")->artefacts()->limit(2)"#,
        true
    ));
}

#[test]
fn extract_cli_payload_unwraps_connection_nodes_through_scopes() {
    let payload = extract_cli_payload(&json!({
        "repo": {
            "project": {
                "artefacts": {
                    "edges": [
                        { "node": { "path": "src/main.rs", "symbolFqn": "main" } },
                        { "node": { "path": "src/lib.rs", "symbolFqn": "answer" } }
                    ]
                }
            }
        }
    }));

    assert_eq!(
        payload,
        json!([
            { "path": "src/main.rs", "symbolFqn": "main" },
            { "path": "src/lib.rs", "symbolFqn": "answer" }
        ])
    );
}

#[test]
fn extract_cli_payload_preserves_typed_project_stage_lists() {
    let payload = extract_cli_payload(&json!({
        "repo": {
            "project": {
                "coverage": [
                    {
                        "artefact": {
                            "name": "run_cli"
                        },
                        "summary": {
                            "uncoveredLineCount": 2
                        }
                    }
                ]
            }
        }
    }));

    assert_eq!(
        payload,
        json!([
            {
                "artefact": {
                    "name": "run_cli"
                },
                "summary": {
                    "uncoveredLineCount": 2
                }
            }
        ])
    );
}

#[test]
fn format_query_output_renders_table_for_dsl_results() {
    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "path": "src/main.rs",
                                "symbolFqn": "main",
                                "chatHistory": {
                                    "edges": [
                                        { "node": { "sessionId": "s1" } },
                                        { "node": { "sessionId": "s2" } }
                                    ]
                                }
                            }
                        }
                    ]
                }
            }
        }),
        false,
        false,
        None,
    )
    .expect("dsl results should render");

    assert!(rendered.contains("| path"));
    assert!(rendered.contains("| symbol_fqn"));
    assert!(rendered.contains("| chat_history"));
    assert!(rendered.contains("src/main.rs"));
    assert!(rendered.contains("[2 entries]"));
}

#[test]
fn format_query_output_emits_compact_json_for_dsl_results() {
    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "path": "src/main.rs",
                                "symbolFqn": "main"
                            }
                        }
                    ]
                }
            }
        }),
        true,
        false,
        None,
    )
    .expect("compact output should render");

    assert_eq!(rendered, r#"[{"path":"src/main.rs","symbolFqn":"main"}]"#);
}

#[test]
fn format_query_output_preserves_non_clone_payload_when_parsed_query_has_no_clones_stage() {
    let parsed = parse_devql_query(r#"repo("bitloops-cli")->artefacts(kind:"function")->limit(5)"#)
        .expect("query parses");

    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "path": "src/main.rs",
                                "symbolFqn": "main"
                            }
                        }
                    ]
                }
            }
        }),
        true,
        false,
        Some(&parsed),
    )
    .expect("compact output should render");

    assert_eq!(rendered, r#"[{"path":"src/main.rs","symbolFqn":"main"}]"#);
}

#[test]
fn format_query_output_keeps_raw_graphql_shape() {
    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "path": "src/main.rs"
                            }
                        }
                    ],
                    "pageInfo": {
                        "hasNextPage": false
                    }
                }
            }
        }),
        false,
        true,
        None,
    )
    .expect("raw graphql should render as json");

    assert!(rendered.contains("\"pageInfo\""));
    assert!(rendered.contains("\"hasNextPage\": false"));
}

#[test]
fn format_query_output_flattens_clone_results_to_user_facing_rows() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones()->limit(5)"#,
    )
    .expect("query parses");

    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "clones": {
                                    "edges": [
                                        {
                                            "node": {
                                                "relationKind": "similar_implementation",
                                                "score": 0.91,
                                                "sourceArtefact": {
                                                    "path": "src/pdf.ts",
                                                    "symbolFqn": "src/pdf.ts::createInvoicePdf"
                                                },
                                                "targetArtefact": {
                                                    "path": "src/render.ts",
                                                    "symbolFqn": "src/render.ts::renderInvoiceDocument"
                                                }
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    ]
                }
            }
        }),
        true,
        false,
        Some(&parsed),
    )
    .expect("clone output should render");

    let value: Value = serde_json::from_str(&rendered).expect("compact clone output is json");
    assert_eq!(
        value,
        json!([
            {
                "from": "src/pdf.ts::createInvoicePdf",
                "to": "src/render.ts::renderInvoiceDocument",
                "relationKind": "similar_implementation",
                "score": 0.91
            }
        ])
    );
}

#[test]
fn format_query_output_flattens_direct_clone_rows_with_user_facing_fallbacks() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones()->limit(5)"#,
    )
    .expect("query parses");

    let rendered = format_query_output(
        &json!([
            {
                "sourceArtefact": {
                    "symbolFqn": "   ",
                    "path": "src/pdf.ts"
                },
                "targetArtefact": {
                    "path": "src/render.ts"
                },
                "relationKind": "exact_duplicate",
                "score": 1.0
            },
            {
                "sourceArtefactId": "artefact::source",
                "targetArtefactId": "   ",
                "relationKind": "similar_implementation"
            },
            {
                "id": "clone::debug"
            },
            "keep-me"
        ]),
        true,
        false,
        Some(&parsed),
    )
    .expect("clone output should render");

    let value: Value = serde_json::from_str(&rendered).expect("compact clone output is valid json");
    assert_eq!(
        value,
        json!([
            {
                "from": "src/pdf.ts",
                "to": "src/render.ts",
                "relationKind": "exact_duplicate",
                "score": 1.0
            },
            {
                "from": "artefact::source",
                "relationKind": "similar_implementation"
            },
            {
                "id": "clone::debug"
            },
            "keep-me"
        ])
    );
}

#[test]
fn format_query_output_keeps_clone_raw_mode_opted_in() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones(raw:true)->limit(5)"#,
    )
    .expect("query parses");

    let rendered = format_query_output(
        &json!({
            "repo": {
                "artefacts": {
                    "edges": [
                        {
                            "node": {
                                "clones": {
                                    "edges": [
                                        {
                                            "node": {
                                                "id": "clone::a::b::similar_implementation",
                                                "sourceArtefactId": "artefact::invoice_pdf",
                                                "targetArtefactId": "artefact::invoice_doc",
                                                "sourceStartLine": 8,
                                                "sourceEndLine": 22,
                                                "targetStartLine": 10,
                                                "targetEndLine": 24,
                                                "relationKind": "similar_implementation",
                                                "score": 0.91,
                                                "metadata": {
                                                    "semanticScore": 0.95,
                                                    "explanation": {
                                                        "labels": ["similar_implementation"]
                                                    }
                                                }
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    ]
                }
            }
        }),
        true,
        false,
        Some(&parsed),
    )
    .expect("raw clone output should render");

    let value: Value = serde_json::from_str(&rendered).expect("compact raw clone output is json");
    assert_eq!(
        value,
        json!([
            {
                "id": "clone::a::b::similar_implementation",
                "sourceArtefactId": "artefact::invoice_pdf",
                "targetArtefactId": "artefact::invoice_doc",
                "sourceStartLine": 8,
                "sourceEndLine": 22,
                "targetStartLine": 10,
                "targetEndLine": 24,
                "relationKind": "similar_implementation",
                "score": 0.91,
                "metadata": {
                    "semanticScore": 0.95,
                    "explanation": {
                        "labels": ["similar_implementation"]
                    }
                }
            }
        ])
    );
}

#[test]
fn format_query_output_preserves_direct_clone_rows_in_raw_mode() {
    let parsed = parse_devql_query(
        r#"repo("bitloops-cli")->artefacts(kind:"function")->clones(raw:true)->limit(5)"#,
    )
    .expect("query parses");

    let rendered = format_query_output(
        &json!([
            {
                "id": "clone::a",
                "sourceArtefactId": "artefact::source",
                "targetArtefactId": "artefact::target",
                "relationKind": "similar_implementation",
                "score": 0.91
            }
        ]),
        true,
        false,
        Some(&parsed),
    )
    .expect("raw clone output should render");

    assert_eq!(
        rendered,
        r#"[{"id":"clone::a","relationKind":"similar_implementation","score":0.91,"sourceArtefactId":"artefact::source","targetArtefactId":"artefact::target"}]"#
    );
}

#[test]
fn format_query_output_renders_clone_summary_object() {
    let rendered = format_query_output(
        &json!({
            "repo": {
                "cloneSummary": {
                    "totalCount": 3,
                    "groups": [
                        {
                            "relationKind": "similar_implementation",
                            "count": 2
                        },
                        {
                            "relationKind": "contextual_neighbor",
                            "count": 1
                        }
                    ]
                }
            }
        }),
        false,
        false,
        None,
    )
    .expect("clone summary should render");

    assert!(rendered.contains("total_count: 3"));
    assert!(rendered.contains("| relation_kind"));
    assert!(rendered.contains("similar_implementation"));
    assert!(rendered.contains("contextual_neighbor"));
}

#[test]
fn format_query_output_renders_clone_summary_arrays_and_empty_groups() {
    let rendered = format_query_output(
        &json!([{
            "total_count": 0,
            "groups": []
        }]),
        false,
        false,
        None,
    )
    .expect("clone summary array should render");

    assert_eq!(rendered, "total_count: 0");
}

#[test]
fn format_query_output_renders_scalar_arrays_as_single_column_tables() {
    let rendered = format_query_output(&json!(["alpha", "beta"]), false, false, None)
        .expect("scalar array should render");

    assert!(rendered.contains("| value"));
    assert!(rendered.contains("alpha"));
    assert!(rendered.contains("beta"));
}

#[test]
fn format_query_output_falls_back_to_json_for_mixed_arrays() {
    let rendered = format_query_output(&json!(["alpha", { "count": 2 }]), false, false, None)
        .expect("mixed array should render");

    assert!(rendered.contains("\"alpha\""));
    assert!(rendered.contains("\"count\": 2"));
}

#[test]
fn format_query_output_preserves_multiple_non_null_root_fields() {
    let rendered = format_query_output(
        &json!({
            "left": { "count": 1 },
            "right": { "count": 2 }
        }),
        false,
        false,
        None,
    )
    .expect("multi-root payload should render");

    assert!(rendered.contains("| left"));
    assert!(rendered.contains("| right"));
    assert!(rendered.contains(r#"{"count":1}"#));
    assert!(rendered.contains(r#"{"count":2}"#));
}

#[test]
fn format_query_output_treats_all_null_objects_as_no_results() {
    let rendered = format_query_output(
        &json!({
            "repo": null
        }),
        false,
        false,
        None,
    )
    .expect("null payload should render");

    assert_eq!(rendered, "No results.");
}

#[test]
fn format_query_output_handles_clone_summary_groups_without_objects() {
    let rendered = format_query_output(
        &json!({
            "repo": {
                "cloneSummary": {
                    "totalCount": 4,
                    "groups": ["unexpected"]
                }
            }
        }),
        false,
        false,
        None,
    )
    .expect("malformed clone summary groups should still render");

    assert_eq!(rendered, "total_count: 4");
}
