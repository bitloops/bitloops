use super::*;

fn artefact_by_symbol_fqn<'a>(
    artefacts: &'a [LanguageArtefact],
    symbol_fqn: &str,
) -> &'a LanguageArtefact {
    artefacts
        .iter()
        .find(|artefact| artefact.symbol_fqn == symbol_fqn)
        .unwrap_or_else(|| panic!("missing artefact {symbol_fqn}"))
}

#[test]
fn symbol_id_stays_stable_when_a_function_moves_lines() {
    let original = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let moved = r#"

export function greet(name: string) {
  return name.trim();
}
"#;

    let original_artefacts = extract_js_ts_artefacts(original, "src/sample.ts").unwrap();
    let moved_artefacts = extract_js_ts_artefacts(moved, "src/sample.ts").unwrap();

    let original_function = artefact_by_symbol_fqn(&original_artefacts, "src/sample.ts::greet");
    let moved_function = artefact_by_symbol_fqn(&moved_artefacts, "src/sample.ts::greet");

    assert_ne!(original_function.start_line, moved_function.start_line);
    assert_eq!(original_function.symbol_fqn, moved_function.symbol_fqn);
    assert_eq!(
        symbol_id_for_artefact(original_function),
        symbol_id_for_artefact(moved_function)
    );
}

#[test]
fn symbol_id_stays_stable_across_formatting_only_edits() {
    let original = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let reformatted = r#"export function greet(name: string) {
    return name.trim( );
}
"#;

    let original_artefacts = extract_js_ts_artefacts(original, "src/sample.ts").unwrap();
    let reformatted_artefacts = extract_js_ts_artefacts(reformatted, "src/sample.ts").unwrap();

    let original_function = artefact_by_symbol_fqn(&original_artefacts, "src/sample.ts::greet");
    let reformatted_function =
        artefact_by_symbol_fqn(&reformatted_artefacts, "src/sample.ts::greet");

    assert_eq!(
        original_function.start_line,
        reformatted_function.start_line
    );
    assert_eq!(
        original_function.symbol_fqn,
        reformatted_function.symbol_fqn
    );
    assert_ne!(original_function.end_byte, reformatted_function.end_byte);
    assert_eq!(
        symbol_id_for_artefact(original_function),
        symbol_id_for_artefact(reformatted_function)
    );
}

#[test]
fn symbol_id_stays_stable_when_only_docstring_changes() {
    let original = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let documented = r#"// greet a user
/* trims surrounding whitespace */
export function greet(name: string) {
  return name.trim();
}
"#;

    let original_artefacts = extract_js_ts_artefacts(original, "src/sample.ts").unwrap();
    let documented_artefacts = extract_js_ts_artefacts(documented, "src/sample.ts").unwrap();

    let original_function = artefact_by_symbol_fqn(&original_artefacts, "src/sample.ts::greet");
    let documented_function = artefact_by_symbol_fqn(&documented_artefacts, "src/sample.ts::greet");

    assert_eq!(
        symbol_id_for_artefact(original_function),
        symbol_id_for_artefact(documented_function)
    );
}

#[test]
fn symbol_id_stays_stable_when_only_modifiers_change() {
    let original = r#"export function greet(name: string) {
  return name.trim();
}
"#;
    let modified = r#"export async function greet(name: string) {
  return name.trim();
}
"#;

    let original_artefacts = extract_js_ts_artefacts(original, "src/sample.ts").unwrap();
    let modified_artefacts = extract_js_ts_artefacts(modified, "src/sample.ts").unwrap();

    let original_function = artefact_by_symbol_fqn(&original_artefacts, "src/sample.ts::greet");
    let modified_function = artefact_by_symbol_fqn(&modified_artefacts, "src/sample.ts::greet");

    assert_eq!(
        symbol_id_for_artefact(original_function),
        symbol_id_for_artefact(modified_function)
    );
}
