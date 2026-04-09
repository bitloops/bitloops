use serde_json::json;

use crate::host::hooks::augmentation::builder::HookAugmentation;

pub fn render_hook_output(hook_name: &str, augmentation: &HookAugmentation) -> Option<String> {
    let hook_event_name = match hook_name {
        crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_START => "SessionStart",
        crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_AGENT => "BeforeAgent",
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
            additional_context:
                "You have DevQL available in this repo. You should leverage it for repo-aware requests."
                    .to_string(),
            targeted: false,
        }
    }

    #[test]
    fn before_agent_renders_additional_context_json() {
        let targeted_augmentation = HookAugmentation {
            additional_context: "You should leverage DevQL for this repo-aware request."
                .to_string(),
            targeted: true,
        };
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_AGENT,
            &targeted_augmentation,
        )
        .expect("rendered");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("BeforeAgent".to_string())
        );
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            serde_json::Value::String(
                "You should leverage DevQL for this repo-aware request.".to_string()
            )
        );
    }

    #[test]
    fn session_start_renders_additional_context_json() {
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_START,
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
            serde_json::Value::String(
                "You have DevQL available in this repo. You should leverage it for repo-aware requests."
                    .to_string()
            )
        );
    }

    #[test]
    fn non_before_agent_hook_returns_none() {
        assert!(
            render_hook_output(
                crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_END,
                &augmentation(),
            )
            .is_none()
        );
    }
}
