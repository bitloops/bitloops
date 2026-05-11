use std::io::{BufRead, BufReader, Cursor};

use serde_json::Value;

const MODEL_KEYS: [&str; 9] = [
    "newModel",
    "model",
    "modelName",
    "model_name",
    "modelSlug",
    "model_slug",
    "modelId",
    "modelID",
    "model_id",
];

pub(crate) fn resolve_interaction_model(model_hint: &str, transcript_path: &str) -> String {
    let model_hint = model_hint.trim();
    if !model_hint.is_empty() {
        return model_hint.to_string();
    }
    extract_model_from_transcript_path(transcript_path)
}

pub(crate) fn resolve_interaction_model_from_bytes(model_hint: &str, transcript: &[u8]) -> String {
    let model_hint = model_hint.trim();
    if !model_hint.is_empty() {
        return model_hint.to_string();
    }
    extract_model_from_transcript_bytes(transcript)
}

pub(crate) fn extract_model_from_transcript_path(transcript_path: &str) -> String {
    if transcript_path.trim().is_empty() {
        return String::new();
    }

    let Ok(data) = std::fs::read(transcript_path) else {
        return String::new();
    };
    extract_model_from_transcript_bytes(&data)
}

pub(crate) fn extract_model_from_transcript_bytes(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    if let Ok((events, _)) =
        crate::adapters::agents::copilot::transcript::parse_events_from_offset(data, 0)
    {
        let model =
            crate::adapters::agents::copilot::transcript::extract_model_from_events(&events);
        if !model.trim().is_empty() {
            return model.trim().to_string();
        }
    }

    extract_model_from_json_bytes(data)
        .or_else(|| extract_model_from_jsonl_bytes(data))
        .unwrap_or_default()
}

fn extract_model_from_json_bytes(data: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(data).ok()?;
    extract_model_from_value(&value)
}

fn extract_model_from_jsonl_bytes(data: &[u8]) -> Option<String> {
    let reader = BufReader::new(Cursor::new(data));
    let mut latest = None;

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(model) = extract_model_from_value(&value) {
            latest = Some(model);
        }
    }

    latest
}

fn extract_model_from_value(value: &Value) -> Option<String> {
    let mut models = Vec::new();
    collect_models(value, &mut models);
    models.into_iter().last()
}

fn collect_models(value: &Value, models: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for key in MODEL_KEYS {
                if let Some(model) = map.get(key).and_then(Value::as_str) {
                    let model = model.trim();
                    if !model.is_empty() {
                        models.push(model.to_string());
                    }
                }
            }

            for nested in map.values() {
                collect_models(nested, models);
            }
        }
        Value::Array(items) => {
            for nested in items {
                collect_models(nested, models);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_model_from_transcript_bytes, resolve_interaction_model,
        resolve_interaction_model_from_bytes,
    };

    #[test]
    fn resolves_explicit_model_hint_first() {
        assert_eq!(
            resolve_interaction_model_from_bytes("gpt-5.4", br#"{"model":"ignored"}"#),
            "gpt-5.4"
        );
    }

    #[test]
    fn extracts_model_from_json_transcript() {
        let transcript = br#"{
            "model": "gemini-2.5-pro",
            "messages": [
                {"type": "user", "content": "hello"}
            ]
        }"#;
        assert_eq!(
            extract_model_from_transcript_bytes(transcript),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn extracts_model_from_jsonl_transcript() {
        let transcript = br#"{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"model":"claude-opus-4-1","content":[{"type":"text","text":"hi"}]}}
"#;
        assert_eq!(
            extract_model_from_transcript_bytes(transcript),
            "claude-opus-4-1"
        );
    }

    #[test]
    fn extracts_model_from_uppercase_id_keys() {
        let transcript = br#"{
            "providerID": "openai",
            "modelID": "gpt-5.4"
        }"#;

        assert_eq!(extract_model_from_transcript_bytes(transcript), "gpt-5.4");
    }

    #[test]
    fn extracts_model_from_copilot_transcript_events() {
        let transcript = br#"{"type":"tool.execution_complete","data":{"model":"gpt-5.2","toolTelemetry":{"properties":{"filePaths":"[]"}}}}
{"type":"session.model_change","data":{"newModel":"gpt-5.4"}}
"#;
        assert_eq!(extract_model_from_transcript_bytes(transcript), "gpt-5.4");
    }

    #[test]
    fn resolve_interaction_model_reads_transcript_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let transcript_path = dir.path().join("transcript.json");
        std::fs::write(&transcript_path, br#"{"modelName":"gemini-2.5-flash"}"#)
            .expect("write transcript");

        assert_eq!(
            resolve_interaction_model("", transcript_path.to_string_lossy().as_ref()),
            "gemini-2.5-flash"
        );
    }
}
