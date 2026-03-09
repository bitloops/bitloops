use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use serde_json::{Map, Value};

const REDACTED: &str = "REDACTED";
const ENTROPY_THRESHOLD: f64 = 4.5;

#[derive(Clone, Copy, Debug)]
struct Region {
    start: usize,
    end: usize,
}

pub fn string(input: &str) -> String {
    let mut regions = entropy_regions(input);
    regions.extend(pattern_regions(input));
    if regions.is_empty() {
        return input.to_string();
    }

    let merged = merge_regions(regions);
    let mut out = String::with_capacity(input.len());
    let mut prev = 0usize;
    for region in merged {
        out.push_str(&input[prev..region.start]);
        out.push_str(REDACTED);
        prev = region.end;
    }
    out.push_str(&input[prev..]);
    out
}

pub fn bytes(input: &[u8]) -> Cow<'_, [u8]> {
    let Ok(text) = std::str::from_utf8(input) else {
        return Cow::Borrowed(input);
    };
    let redacted = string(text);
    if redacted == text {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(redacted.into_bytes())
    }
}

pub fn jsonl_bytes(input: &[u8]) -> Result<Cow<'_, [u8]>> {
    let Ok(text) = std::str::from_utf8(input) else {
        return Ok(Cow::Borrowed(input));
    };
    let redacted = jsonl_content(text)?;
    if redacted == text {
        Ok(Cow::Borrowed(input))
    } else {
        Ok(Cow::Owned(redacted.into_bytes()))
    }
}

pub fn jsonl_content(content: &str) -> Result<String> {
    let mut out = String::with_capacity(content.len());
    for (index, line) in content.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push_str(line);
            continue;
        }

        let parsed = match serde_json::from_str::<Value>(trimmed) {
            Ok(parsed) => parsed,
            Err(_) => {
                out.push_str(&string(line));
                continue;
            }
        };

        let replacements = collect_jsonl_replacements(&parsed);
        if replacements.is_empty() {
            out.push_str(line);
            continue;
        }

        let mut result = line.to_string();
        for (original, replacement) in replacements {
            let orig_json = json_encode_string(&original)?;
            let repl_json = json_encode_string(&replacement)?;
            result = result.replace(&orig_json, &repl_json);
        }
        out.push_str(&result);
    }
    Ok(out)
}

pub(crate) fn collect_jsonl_replacements(value: &Value) -> Vec<(String, String)> {
    let mut replacements = Vec::new();
    let mut seen = HashSet::new();
    collect_jsonl_replacements_inner(value, &mut seen, &mut replacements);
    replacements
}

pub(crate) fn should_skip_jsonl_field(key: &str) -> bool {
    if key == "signature" {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    lower.ends_with("id") || lower.ends_with("ids")
}

pub(crate) fn should_skip_jsonl_object(obj: &Map<String, Value>) -> bool {
    let Some(value) = obj.get("type") else {
        return false;
    };
    let Some(kind) = value.as_str() else {
        return false;
    };
    kind.starts_with("image") || kind == "base64"
}

pub(crate) fn shannon_entropy(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }

    let mut freq: HashMap<u8, usize> = HashMap::new();
    for byte in value.bytes() {
        *freq.entry(byte).or_insert(0) += 1;
    }

    let len = value.len() as f64;
    let mut entropy = 0.0f64;
    for count in freq.into_values() {
        let p = count as f64 / len;
        entropy -= p * p.log2();
    }
    entropy
}

fn collect_jsonl_replacements_inner(
    value: &Value,
    seen: &mut HashSet<String>,
    out: &mut Vec<(String, String)>,
) {
    match value {
        Value::Object(map) => {
            if should_skip_jsonl_object(map) {
                return;
            }
            for (key, child) in map {
                if should_skip_jsonl_field(key) {
                    continue;
                }
                collect_jsonl_replacements_inner(child, seen, out);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_jsonl_replacements_inner(child, seen, out);
            }
        }
        Value::String(value) => {
            let redacted = string(value);
            if redacted != *value && seen.insert(value.clone()) {
                out.push((value.clone(), redacted));
            }
        }
        _ => {}
    }
}

fn json_encode_string(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn secret_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9/+_=-]{10,}").expect("secret pattern regex must compile")
    })
}

fn aws_access_key_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?:A3T|AKIA|ASIA|AGPA|AIDA|AROA|AIPA)[A-Z0-9]{16}")
            .expect("aws access key regex must compile")
    })
}

fn entropy_regions(input: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    for capture in secret_pattern().find_iter(input) {
        let candidate = &input[capture.start()..capture.end()];
        if shannon_entropy(candidate) > ENTROPY_THRESHOLD {
            regions.push(Region {
                start: capture.start(),
                end: capture.end(),
            });
        }
    }
    regions
}

fn pattern_regions(input: &str) -> Vec<Region> {
    aws_access_key_pattern()
        .find_iter(input)
        .map(|capture| Region {
            start: capture.start(),
            end: capture.end(),
        })
        .collect()
}

fn merge_regions(mut regions: Vec<Region>) -> Vec<Region> {
    regions.sort_by_key(|r| r.start);
    let mut merged: Vec<Region> = Vec::with_capacity(regions.len());
    for region in regions {
        if let Some(last) = merged.last_mut()
            && region.start <= last.end
        {
            if region.end > last.end {
                last.end = region.end;
            }
            continue;
        }
        merged.push(region);
    }
    merged
}
