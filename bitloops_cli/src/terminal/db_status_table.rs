use std::env;

use crate::engine::db_status::{DatabaseConnectionStatus, DatabaseStatusRow};

pub fn print_db_status_table(rows: &[DatabaseStatusRow]) {
    const DB_WIDTH: usize = 10;
    const STATUS_WIDTH: usize = 23;

    println!("DB Status");
    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = DB_WIDTH,
        status_width = STATUS_WIDTH
    );
    println!(
        "| {:<db_width$} | {:<status_width$} |",
        "DB",
        "Status",
        db_width = DB_WIDTH,
        status_width = STATUS_WIDTH
    );
    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = DB_WIDTH,
        status_width = STATUS_WIDTH
    );

    for row in rows {
        let raw_status = format!(
            "{:<status_width$}",
            row.status.label(),
            status_width = STATUS_WIDTH
        );
        let colored_status = colorize_status_label(row.status, &raw_status);
        println!(
            "| {:<db_width$} | {} |",
            row.db,
            colored_status,
            db_width = DB_WIDTH
        );
    }

    println!(
        "+-{:-<db_width$}-+-{:-<status_width$}-+",
        "",
        "",
        db_width = DB_WIDTH,
        status_width = STATUS_WIDTH
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
