use serde_json::json;

use crate::host::hooks::augmentation::builder::HookAugmentation;

pub fn render_hook_output(hook_name: &str, augmentation: &HookAugmentation) -> Option<String> {
    if hook_name != crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SESSION_START {
        return None;
    }

    Some(
        json!({
            "additional_context": augmentation.additional_context,
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
    fn session_start_renders_additional_context_json() {
        let rendered = render_hook_output(
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SESSION_START,
            &augmentation(),
        )
        .expect("rendered");

        let value: serde_json::Value = serde_json::from_str(&rendered).expect("json");
        assert_eq!(
            value["additional_context"],
            serde_json::Value::String(
                "You have DevQL available in this repo. You should leverage it for repo-aware requests."
                    .to_string()
            )
        );
    }

    #[test]
    fn non_session_start_hook_returns_none() {
        assert!(
            render_hook_output(
                crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_STOP,
                &augmentation(),
            )
            .is_none()
        );
    }
}
