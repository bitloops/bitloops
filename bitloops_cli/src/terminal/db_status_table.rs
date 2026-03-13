use std::env;
use std::fmt::Write as _;

use crate::engine::db_status::{DatabaseConnectionStatus, DatabaseStatusRow};

pub fn print_db_status_table(rows: &[DatabaseStatusRow]) {
    print!("{}", render_db_status_table(rows));
}

fn render_db_status_table(rows: &[DatabaseStatusRow]) -> String {
    let db_width = rows
        .iter()
        .map(|row| row.db.len())
        .max()
        .unwrap_or(2)
        .max("DB".len());
    let status_width = rows
        .iter()
        .map(|row| row.status.label().len())
        .max()
        .unwrap_or(6)
        .max("Status".len());

    let mut out = String::new();
    out.push('\n');
    writeln!(
        &mut out,
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    )
    .expect("writing to string should succeed");
    writeln!(
        &mut out,
        "| {:<db_width$} | {:<status_width$} |",
        "DB",
        "Status",
        db_width = db_width,
        status_width = status_width
    )
    .expect("writing to string should succeed");
    writeln!(
        &mut out,
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    )
    .expect("writing to string should succeed");

    for row in rows {
        let raw_status = format!(
            "{:<status_width$}",
            row.status.label(),
            status_width = status_width
        );
        let colored_status = colorize_status_label(row.status, &raw_status);
        writeln!(
            &mut out,
            "| {:<db_width$} | {} |",
            row.db,
            colored_status,
            db_width = db_width
        )
        .expect("writing to string should succeed");
    }

    writeln!(
        &mut out,
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    )
    .expect("writing to string should succeed");
    out
}

fn colorize_status_label(status: DatabaseConnectionStatus, text: &str) -> String {
    if !should_use_color_output() {
        return text.to_string();
    }

    let code = match status {
        DatabaseConnectionStatus::Connected => "32",
        DatabaseConnectionStatus::CouldNotAuthenticate
        | DatabaseConnectionStatus::CouldNotReachDb
        | DatabaseConnectionStatus::Error => "31",
        DatabaseConnectionStatus::NotConfigured => "33",
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn should_use_color_output() -> bool {
    env::var_os("NO_COLOR").is_none() && env::var("ACCESSIBLE").is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_env_vars;

    fn test_rows() -> [DatabaseStatusRow; 2] {
        [
            DatabaseStatusRow {
                db: "SQLite",
                status: DatabaseConnectionStatus::Connected,
            },
            DatabaseStatusRow {
                db: "ClickHouse",
                status: DatabaseConnectionStatus::NotConfigured,
            },
        ]
    }

    #[test]
    fn render_db_status_table_omits_ansi_when_colour_is_disabled() {
        with_env_vars(&[("NO_COLOR", Some("1")), ("ACCESSIBLE", None)], || {
            let rendered = render_db_status_table(&test_rows());

            assert!(rendered.starts_with('\n'));
            assert!(rendered.contains("| DB         | Status         |"));
            assert!(rendered.contains("| SQLite     | Connected      |"));
            assert!(rendered.contains("| ClickHouse | Not configured |"));
            assert!(!rendered.contains("\x1b["));
        });
    }

    #[test]
    fn colorize_status_label_uses_expected_ansi_codes() {
        with_env_vars(&[("NO_COLOR", None), ("ACCESSIBLE", None)], || {
            assert_eq!(
                colorize_status_label(DatabaseConnectionStatus::Connected, "Connected"),
                "\x1b[32mConnected\x1b[0m"
            );
            assert_eq!(
                colorize_status_label(DatabaseConnectionStatus::NotConfigured, "Not configured"),
                "\x1b[33mNot configured\x1b[0m"
            );
            assert_eq!(
                colorize_status_label(
                    DatabaseConnectionStatus::CouldNotReachDb,
                    "Could not reach DB"
                ),
                "\x1b[31mCould not reach DB\x1b[0m"
            );
        });
    }

    #[test]
    fn accessible_mode_disables_colour_output() {
        with_env_vars(&[("NO_COLOR", None), ("ACCESSIBLE", Some("1"))], || {
            assert!(!should_use_color_output());
        });
    }
}
