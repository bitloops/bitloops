use super::*;

#[test]
fn artefact_selector_accepts_symbol_fqn_or_path_modes() {
    let symbol = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        fuzzy_name: None,
        path: None,
        lines: None,
    };
    assert_eq!(
        symbol.selection_mode().expect("symbol selector"),
        ArtefactSelectorMode::SymbolFqn("src/main.rs::main".to_string())
    );

    let path = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: None,
        path: Some("src/main.rs".to_string()),
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    };
    assert_eq!(
        path.selection_mode().expect("path selector"),
        ArtefactSelectorMode::Path {
            path: "src/main.rs".to_string(),
            lines: Some(LineRangeInput { start: 20, end: 25 }),
        }
    );
}

#[test]
fn artefact_selector_accepts_fuzzy_name_mode() {
    let fuzzy = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: Some("payLater()".to_string()),
        path: None,
        lines: None,
    };

    assert_eq!(
        fuzzy.selection_mode().expect("fuzzy selector"),
        ArtefactSelectorMode::FuzzyName("payLater()".to_string())
    );
}

#[test]
fn artefact_selector_rejects_invalid_combinations() {
    let err = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        fuzzy_name: None,
        path: Some("src/main.rs".to_string()),
        lines: None,
    }
    .selection_mode()
    .expect_err("mixed selector should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: None,
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("lines without path should fail");
    assert!(
        err.message
            .contains("requires `path` when `lines` is provided")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: Some("  ".to_string()),
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("blank fuzzy selector should fail");
    assert!(err.message.contains("non-empty `fuzzyName`"));

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: Some("payLater".to_string()),
        path: Some("src/main.rs".to_string()),
        lines: None,
    }
    .selection_mode()
    .expect_err("fuzzy selector mixed with path should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: Some("payLater".to_string()),
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("fuzzy selector mixed with lines should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: Some("src/main.rs::main".to_string()),
        fuzzy_name: None,
        path: None,
        lines: Some(LineRangeInput { start: 20, end: 25 }),
    }
    .selection_mode()
    .expect_err("symbol selector mixed with lines should fail");
    assert!(
        err.message
            .contains("allows exactly one of `symbolFqn`, `fuzzyName`, or `path`/`lines`")
    );

    let err = ArtefactSelectorInput {
        symbol_fqn: None,
        fuzzy_name: None,
        path: None,
        lines: None,
    }
    .selection_mode()
    .expect_err("empty selector should fail");
    assert!(err.message.contains("requires exactly one selector mode"));
}
