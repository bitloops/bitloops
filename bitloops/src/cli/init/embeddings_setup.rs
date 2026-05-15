use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::embeddings::{
    EmbeddingsInstallState, EmbeddingsRuntime, inspect_embeddings_install_state,
};
use crate::cli::telemetry_consent;
use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};

use super::InitArgs;

pub(crate) const NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR: &str = "`bitloops init --install-default-daemon` requires an explicit embeddings choice when not running interactively. Pass `--embeddings-runtime local`, `--embeddings-runtime platform`, or `--no-embeddings`.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitEmbeddingsSetupSelection {
    Existing,
    Cloud,
    Local,
    Skip,
}

pub(crate) fn should_install_embeddings_during_init(
    repo_root: &Path,
    args: &InitArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if args.no_embeddings {
        return Ok(InitEmbeddingsSetupSelection::Skip);
    }

    if let Some(runtime) = args.embeddings_runtime {
        return Ok(match runtime {
            EmbeddingsRuntime::Local => InitEmbeddingsSetupSelection::Local,
            EmbeddingsRuntime::Platform => InitEmbeddingsSetupSelection::Cloud,
        });
    }

    if !args.install_default_daemon {
        return Ok(InitEmbeddingsSetupSelection::Skip);
    }

    if !matches!(
        inspect_embeddings_install_state(repo_root),
        EmbeddingsInstallState::NotConfigured
    ) {
        return Ok(InitEmbeddingsSetupSelection::Existing);
    }

    if !telemetry_consent::can_prompt_interactively() {
        bail!(NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR);
    }

    prompt_install_embeddings_setup_selection(out, input)
}

pub(crate) fn prompt_install_embeddings_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    if can_use_terminal_picker() {
        return prompt_install_embeddings_setup_selection_with_picker(out);
    }

    prompt_install_embeddings_setup_selection_with_text_input(out, input)
}

fn prompt_install_embeddings_setup_selection_with_picker(
    out: &mut dyn Write,
) -> Result<InitEmbeddingsSetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops Cloud (recommended)",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local embeddings",
            vec!["Runs on your machine (~4GB RAM, GPU recommended).".to_string()],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "Configure embeddings",
        &[
            "Embeddings power semantic search across your codebase".to_string(),
            "(e.g. “find where authentication is handled”).".to_string(),
            String::new(),
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
                .to_string(),
        ],
        &options,
        0,
        &[],
    )?;

    Ok(match selection {
        0 => InitEmbeddingsSetupSelection::Cloud,
        1 => InitEmbeddingsSetupSelection::Local,
        2 => InitEmbeddingsSetupSelection::Skip,
        _ => unreachable!("terminal picker returned invalid embeddings selection"),
    })
}

fn prompt_install_embeddings_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<InitEmbeddingsSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure embeddings")?;
    writeln!(out)?;
    writeln!(out, "Embeddings power semantic search across your codebase")?;
    writeln!(out, "(e.g. “find where authentication is handled”).")?;
    writeln!(out)?;
    writeln!(
        out,
        "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
    )?;
    writeln!(out)?;
    writeln!(out, "1. Bitloops Cloud (recommended)")?;
    writeln!(out, "   Fast setup. No local compute required.")?;
    writeln!(out, "2. Local embeddings")?;
    writeln!(out, "   Runs on your machine (~4GB RAM, GPU recommended).")?;
    writeln!(out, "3. Skip for now")?;

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading init embeddings setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "cloud" | "bitloops" => return Ok(InitEmbeddingsSetupSelection::Cloud),
            "2" | "local" => return Ok(InitEmbeddingsSetupSelection::Local),
            "3" | "skip" | "later" | "none" => {
                return Ok(InitEmbeddingsSetupSelection::Skip);
            }
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}
