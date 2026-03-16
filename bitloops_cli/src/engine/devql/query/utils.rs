fn extract_chat_messages_from_transcript(transcript: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let Some(text) = extract_message_text(&value) else {
            continue;
        };
        let role = extract_message_role(&value).unwrap_or_else(|| "unknown".to_string());
        messages.push(json!({
            "role": role,
            "text": text,
        }));
    }
    messages
}

fn extract_message_role(value: &Value) -> Option<String> {
    value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/role").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn extract_message_text(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(flatten_text_value)
        .or_else(|| value.get("content").and_then(flatten_text_value))
        .or_else(|| value.get("text").and_then(flatten_text_value))
}

fn flatten_text_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = flatten_text_value(item) {
                    parts.push(text);
                }
            }

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Object(map) => map
            .get("text")
            .and_then(flatten_text_value)
            .or_else(|| map.get("content").and_then(flatten_text_value))
            .or_else(|| map.get("input").and_then(flatten_text_value)),
        _ => None,
    }
}

fn resolve_repo_id_for_query(cfg: &DevqlConfig, requested_repo: Option<&str>) -> String {
    let Some(repo) = requested_repo else {
        return cfg.repo.repo_id.clone();
    };

    let normalized = repo.trim();
    if normalized.is_empty() {
        return cfg.repo.repo_id.clone();
    }

    let local_candidates = [
        cfg.repo.name.as_str(),
        cfg.repo.identity.as_str(),
        &format!("{}/{}", cfg.repo.organization, cfg.repo.name),
    ];

    if local_candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(normalized))
    {
        return cfg.repo.repo_id.clone();
    }

    deterministic_uuid(&format!("repo://{normalized}"))
}

fn sql_string_list_ch(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn sql_string_list_pg(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn project_rows(rows: Vec<Value>, fields: &[String]) -> Vec<Value> {
    if fields.is_empty() {
        return rows;
    }

    if fields.len() == 1 && fields[0].trim() == "count()" {
        return vec![json!({ "count": rows.len() })];
    }

    let mut projected = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(obj) = row.as_object() {
            let mut out = Map::new();
            for field in fields {
                if field.trim() == "count()" {
                    continue;
                }
                if let Some(value) = lookup_nested_field(obj, field.trim()) {
                    out.insert(field.trim().to_string(), value.clone());
                } else {
                    out.insert(field.trim().to_string(), Value::Null);
                }
            }
            projected.push(Value::Object(out));
        } else {
            projected.push(row);
        }
    }
    projected
}

fn lookup_nested_field<'a>(obj: &'a Map<String, Value>, field: &str) -> Option<&'a Value> {
    if !field.contains('.') {
        return obj.get(field);
    }

    let mut current: Option<&Value> = None;
    for (index, part) in field.split('.').enumerate() {
        if index == 0 {
            current = obj.get(part);
        } else {
            current = current.and_then(Value::as_object).and_then(|m| m.get(part));
        }
    }
    current
}
