use std::env;

use crate::engine::db_status::{DatabaseConnectionStatus, DatabaseStatusRow};

pub fn print_db_status_table(rows: &[DatabaseStatusRow]) {
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

    println!();
    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    );
    println!(
        "| {:<db_width$} | {:<status_width$} |",
        "DB",
        "Status",
        db_width = db_width,
        status_width = status_width
    );
    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    );

    for row in rows {
        let raw_status = format!(
            "{:<status_width$}",
            row.status.label(),
            status_width = status_width
        );
        let colored_status = colorize_status_label(row.status, &raw_status);
        println!(
            "| {:<db_width$} | {} |",
            row.db,
            colored_status,
            db_width = db_width
        );
    }

    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = db_width,
        status_width = status_width
    );
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
