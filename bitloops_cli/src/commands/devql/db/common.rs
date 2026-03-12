fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
}

fn esc_ch(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn esc_duck(value: &str) -> String {
    value.replace('\'', "''")
}

fn parse_json_string_array(raw: String) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Array(vec![]);
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(values)) => Value::Array(values),
        _ => Value::Array(vec![]),
    }
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn build_path_candidates(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        out.push(raw.to_string());
    }

    let normalized = normalize_repo_path(raw);
    if !normalized.is_empty() {
        out.push(normalized.clone());
        out.push(format!("./{normalized}"));
    }

    out.sort();
    out.dedup();
    out
}

fn sql_path_candidates_clause(column: &str, candidates: &[String]) -> String {
    if candidates.is_empty() {
        return "1=0".to_string();
    }

    candidates
        .iter()
        .map(|candidate| format!("{column} = '{}'", esc_pg(candidate)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn format_ch_array(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }

    let parts = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

fn glob_to_sql_like(glob: &str) -> String {
    glob.replace("**", "%").replace('*', "%")
}

fn deterministic_uuid(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = format!("{:x}", hasher.finalize());

    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
