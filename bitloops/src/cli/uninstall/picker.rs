use std::collections::BTreeSet;
use std::io::Write;

use anyhow::{Result, bail};

use super::targets::{ALL_TARGETS, UninstallTarget};
use crate::cli::terminal_picker::{MultiSelectOption, prompt_multi_select};

pub(super) fn prompt_select_targets(
    out: &mut dyn Write,
) -> Result<Option<BTreeSet<UninstallTarget>>> {
    prompt_select_targets_impl(out)
}

fn prompt_select_targets_impl(out: &mut dyn Write) -> Result<Option<BTreeSet<UninstallTarget>>> {
    let options = ALL_TARGETS
        .iter()
        .map(|target| MultiSelectOption::new(target.picker_label(), Vec::new(), false))
        .collect::<Vec<_>>();

    match prompt_multi_select(
        out,
        "Select what to uninstall:",
        &["Use space to select, enter to confirm.".to_string()],
        &options,
        &["x toggle • ↑/↓ move • enter submit • ctrl+a all".to_string()],
    ) {
        Ok(selected_indexes) => {
            let selected_targets = selected_indexes
                .into_iter()
                .map(|index| ALL_TARGETS[index])
                .collect::<BTreeSet<_>>();
            if selected_targets.is_empty() {
                bail!("no uninstall targets selected");
            }
            Ok(Some(selected_targets))
        }
        Err(err) if err.to_string() == "cancelled by user" => Ok(None),
        Err(err) if err.to_string() == "no options selected" => {
            bail!("no uninstall targets selected")
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::terminal_picker::with_multi_select_hook;

    #[test]
    fn picker_toggle_and_submit_selects_requested_targets() {
        let mut out = Vec::new();
        let selected = with_multi_select_hook(
            |_options, _cursor| Ok(vec![0, 2]),
            || {
                prompt_select_targets_impl(&mut out)
                    .expect("selection should succeed")
                    .expect("selection should not be cancelled")
            },
        );

        assert_eq!(
            selected,
            BTreeSet::from([UninstallTarget::AgentHooks, UninstallTarget::GitHooks])
        );
    }

    #[test]
    fn picker_ctrl_a_selects_everything() {
        let mut out = Vec::new();
        let selected = with_multi_select_hook(
            |_options, _cursor| Ok((0..ALL_TARGETS.len()).collect()),
            || {
                prompt_select_targets_impl(&mut out)
                    .expect("selection should succeed")
                    .expect("selection should not be cancelled")
            },
        );

        assert_eq!(selected, BTreeSet::from(ALL_TARGETS));
    }

    #[test]
    fn picker_cancel_returns_none() {
        let mut out = Vec::new();
        let selected = with_multi_select_hook(
            |_options, _cursor| bail!("cancelled by user"),
            || prompt_select_targets_impl(&mut out).expect("cancel should not error"),
        );
        assert_eq!(selected, None);
    }

    #[test]
    fn picker_submit_without_selection_errors() {
        let mut out = Vec::new();
        let err = with_multi_select_hook(
            |_options, _cursor| Ok(Vec::new()),
            || prompt_select_targets_impl(&mut out).expect_err("empty selection should error"),
        );
        assert!(format!("{err:#}").contains("no uninstall targets selected"));
    }
}
