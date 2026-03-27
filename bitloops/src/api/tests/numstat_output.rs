use super::*;

#[test]
fn parse_numstat_output_parses_normal_line() {
    let raw = "5\t2\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 2)));
}

#[test]
fn parse_numstat_output_treats_binary_as_zero() {
    let raw = "-\t-\tassets/logo.png\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("assets/logo.png"), Some(&(0, 0)));
}

#[test]
fn parse_numstat_output_ignores_malformed_lines() {
    let raw = "not-a-valid-line\n5\t2\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 2)));
}

#[test]
fn parse_numstat_output_accumulates_duplicate_paths() {
    let raw = "3\t1\tsrc/a.rs\n2\t0\tsrc/a.rs\n";
    let stats = parse_numstat_output(raw);
    assert_eq!(stats.get("src/a.rs"), Some(&(5, 1)));
}
