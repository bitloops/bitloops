use serde_json::json;

use crate::host::hooks::augmentation::builder::HookAugmentation;

pub fn render_hook_output(hook_name: &str, augmentation: &HookAugmentation) -> Option<String> {
    if hook_name != crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_AGENT {
        return None;
    }

    Some(
        json!({
            "hookSpecificOutput": {
                "hookEventName": "BeforeAgent",
                "additionalContext": augmentation.additional_context,
            }
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn augmentation() -> HookAugmentation {
        HookAugmentation {
            additional_context: "Use selectArtefacts first.".to_string(),
            targeted: true,
        }
    }

    #[test]
    fn before_agent_renders_additional_context_json() {
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_AGENT,
            &augmentation(),
        )
        .expect("rendered");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("BeforeAgent".to_string())
        );
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            serde_json::Value::String("Use selectArtefacts first.".to_string())
        );
    }

    #[test]
    fn non_before_agent_hook_returns_none() {
        assert!(
            render_hook_output(
                crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_START,
                &augmentation(),
            )
            .is_none()
        );
    }
}
