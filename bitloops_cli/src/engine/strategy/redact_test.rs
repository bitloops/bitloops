use std::borrow::Cow;

use regex::Regex;
use serde_json::json;

use super::redact;

const HIGH_ENTROPY_SECRET: &str = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";

#[test]
#[allow(non_snake_case)]
fn TestBytes_NoSecrets() {
    let input = b"hello world, this is normal text";
    let result = redact::bytes(input);
    assert_eq!(result.as_ref(), input);
    assert!(
        matches!(result, Cow::Borrowed(_)),
        "expected borrowed bytes when no redaction is needed"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestBytes_WithSecret() {
    let input = format!("my key is {HIGH_ENTROPY_SECRET} ok").into_bytes();
    let result = redact::bytes(&input);
    assert_eq!(result.as_ref(), b"my key is REDACTED ok");
}

#[test]
#[allow(non_snake_case)]
fn TestJSONLBytes_NoSecrets() {
    let input = br#"{"type":"text","content":"hello"}"#;
    let result = redact::jsonl_bytes(input).expect("jsonl_bytes should succeed");
    assert_eq!(result.as_ref(), input);
    assert!(
        matches!(result, Cow::Borrowed(_)),
        "expected borrowed bytes when no redaction is needed"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestJSONLBytes_WithSecret() {
    let input = format!(r#"{{"type":"text","content":"key={HIGH_ENTROPY_SECRET}"}}"#).into_bytes();
    let result = redact::jsonl_bytes(&input).expect("jsonl_bytes should succeed");
    assert_eq!(result.as_ref(), br#"{"type":"text","content":"REDACTED"}"#);
}

#[test]
#[allow(non_snake_case)]
fn TestJSONLContent_TopLevelArray() {
    let input = format!(r#"["{HIGH_ENTROPY_SECRET}","normal text"]"#);
    let result = redact::jsonl_content(&input).expect("jsonl_content should succeed");
    assert_eq!(result, r#"["REDACTED","normal text"]"#);
}

#[test]
#[allow(non_snake_case)]
fn TestJSONLContent_TopLevelArrayNoSecrets() {
    let input = r#"["hello","world"]"#;
    let result = redact::jsonl_content(input).expect("jsonl_content should succeed");
    assert_eq!(result, input);
}

#[test]
#[allow(non_snake_case)]
fn TestJSONLContent_InvalidJSONLine() {
    let input = format!(r#"{{"type":"text", "invalid {HIGH_ENTROPY_SECRET} json"#);
    let result = redact::jsonl_content(&input).expect("jsonl_content should succeed");
    assert_eq!(result, r#"{"type":"text", "invalid REDACTED json"#);
}

#[test]
#[allow(non_snake_case)]
fn TestCollectJSONLReplacements_Succeeds() {
    let value = json!({
        "content": format!("token={HIGH_ENTROPY_SECRET}")
    });
    let replacements = redact::collect_jsonl_replacements(&value);
    let expected = vec![(
        format!("token={HIGH_ENTROPY_SECRET}"),
        "REDACTED".to_string(),
    )];
    assert_eq!(replacements, expected);
}

#[test]
#[allow(non_snake_case)]
fn TestShouldSkipJSONLField() {
    let tests = vec![
        ("id", true),
        ("session_id", true),
        ("sessionId", true),
        ("checkpoint_id", true),
        ("checkpointID", true),
        ("userId", true),
        ("ids", true),
        ("session_ids", true),
        ("userIds", true),
        ("signature", true),
        ("content", false),
        ("type", false),
        ("name", false),
        ("video", false),
        ("identify", false),
        ("signatures", false),
        ("signal_data", false),
        ("consideration", false),
    ];

    for (key, expected) in tests {
        let got = redact::should_skip_jsonl_field(key);
        assert_eq!(
            got, expected,
            "should_skip_jsonl_field({key:?}) = {got}, want {expected}"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestShouldSkipJSONLField_RedactionBehavior() {
    let value = json!({
        "session_id": HIGH_ENTROPY_SECRET,
        "content": HIGH_ENTROPY_SECRET
    });
    let replacements = redact::collect_jsonl_replacements(&value);
    assert_eq!(replacements.len(), 1, "expected one replacement");
    assert_eq!(replacements[0].0, HIGH_ENTROPY_SECRET);
}

#[test]
#[allow(non_snake_case)]
fn TestString_PatternDetection() {
    let tests = vec![
        (
            "AWS access key (entropy ~3.9, below 4.5 threshold)",
            "key=AKIAYRWQG5EJLPZLBYNP",
            "key=REDACTED",
        ),
        (
            "two AWS keys separated by space produce two REDACTED tokens",
            "key=AKIAYRWQG5EJLPZLBYNP AKIAYRWQG5EJLPZLBYNP",
            "key=REDACTED REDACTED",
        ),
        (
            "adjacent AWS keys without separator merge into single REDACTED",
            "key=AKIAYRWQG5EJLPZLBYNPAKIAYRWQG5EJLPZLBYNP",
            "key=REDACTED",
        ),
    ];
    let token_pattern = Regex::new(r"[A-Za-z0-9/+_=-]{10,}").expect("token regex");

    for (name, input, expected) in tests {
        for m in token_pattern.find_iter(input) {
            let entropy = redact::shannon_entropy(m.as_str());
            assert!(
                entropy <= 4.5,
                "case {name}: token {:?} has entropy {entropy:.2} > 4.5",
                m.as_str()
            );
        }
        let got = redact::string(input);
        assert_eq!(got, expected, "case {name}");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestShouldSkipJSONLObject() {
    let tests = vec![
        (json!({"type":"image", "data":"base64data"}), true),
        (json!({"type":"text", "content":"hello"}), false),
        (json!({"content":"hello"}), false),
        (json!({"type":42}), false),
        (json!({"type":"image_url"}), true),
        (json!({"type":"base64"}), true),
    ];

    for (value, expected) in tests {
        let obj = value
            .as_object()
            .expect("test fixture should be a JSON object");
        let got = redact::should_skip_jsonl_object(obj);
        assert_eq!(got, expected, "should_skip_jsonl_object({value})");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestShouldSkipJSONLObject_RedactionBehavior() {
    let image_obj = json!({
        "type": "image",
        "data": HIGH_ENTROPY_SECRET
    });
    let replacements = redact::collect_jsonl_replacements(&image_obj);
    assert!(
        replacements.is_empty(),
        "image objects should not produce replacements"
    );

    let text_obj = json!({
        "type": "text",
        "content": HIGH_ENTROPY_SECRET
    });
    let replacements = redact::collect_jsonl_replacements(&text_obj);
    assert_eq!(
        replacements,
        vec![(HIGH_ENTROPY_SECRET.to_string(), "REDACTED".to_string())]
    );
}
