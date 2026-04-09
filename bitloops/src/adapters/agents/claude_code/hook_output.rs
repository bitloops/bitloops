use serde_json::json;

use crate::host::hooks::augmentation::builder::HookAugmentation;

pub fn render_hook_output(hook_name: &str, augmentation: &HookAugmentation) -> Option<String> {
    let hook_event_name = match hook_name {
        crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_SESSION_START => "SessionStart",
        crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_USER_PROMPT_SUBMIT => {
            "UserPromptSubmit"
        }
        _ => return None,
    };

    Some(
        json!({
            "hookSpecificOutput": {
                "hookEventName": hook_event_name,
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
    fn user_prompt_submit_renders_additional_context_json() {
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_USER_PROMPT_SUBMIT,
            &augmentation(),
        )
        .expect("rendered");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("UserPromptSubmit".to_string())
        );
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            serde_json::Value::String("Use selectArtefacts first.".to_string())
        );
    }

    #[test]
    fn session_start_renders_additional_context_json() {
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_SESSION_START,
            &augmentation(),
        )
        .expect("rendered");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("SessionStart".to_string())
        );
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            serde_json::Value::String("Use selectArtefacts first.".to_string())
        );
    }

    #[test]
    fn non_prompt_hook_returns_none() {
        assert!(
            render_hook_output(
                crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_STOP,
                &augmentation(),
            )
            .is_none()
        );
    }
}
