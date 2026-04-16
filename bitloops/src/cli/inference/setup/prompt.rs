use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};

use super::types::SummarySetupSelection;

pub(crate) fn prompt_summary_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
    default_to_local_when_noninteractive: bool,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    if !interactive {
        return Ok(if cloud_logged_in {
            SummarySetupSelection::Cloud
        } else if default_to_local_when_noninteractive {
            SummarySetupSelection::Local
        } else {
            SummarySetupSelection::Skip
        });
    }

    if can_use_terminal_picker() {
        return prompt_summary_setup_selection_with_picker(out, cloud_logged_in);
    }

    prompt_summary_setup_selection_with_text_input(out, input, cloud_logged_in)
}

fn prompt_summary_setup_selection_with_picker(
    out: &mut dyn Write,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops cloud (recommended)",
            vec![
                "Requires you to create or use your free Bitloops account. No local model needed."
                    .to_string(),
            ],
        ),
        SingleSelectOption::new(
            "Local Ollama",
            vec![
                "No code leaves your machine but requires RAM >32GB and GPU acceleration (64GB+ recommended)."
                    .to_string(),
            ],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];
    let mut footer = Vec::new();
    if !cloud_logged_in {
        footer.push(
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
                .to_string(),
        );
    }

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "How would you like Bitloops to configure semantic summaries?",
        &options,
        0,
        &footer,
    )?;

    Ok(match selection {
        0 => SummarySetupSelection::Cloud,
        1 => SummarySetupSelection::Local,
        2 => SummarySetupSelection::Skip,
        _ => unreachable!("terminal picker returned invalid summary selection"),
    })
}

fn prompt_summary_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    writeln!(out)?;
    writeln!(
        out,
        "How would you like Bitloops to configure semantic summaries?"
    )?;
    writeln!(out, "1. Bitloops cloud (recommended)")?;
    writeln!(
        out,
        "   Requires you to create or use your free Bitloops account. No local model needed."
    )?;
    writeln!(out, "2. Local Ollama")?;
    writeln!(
        out,
        "   No code leaves your machine but requires RAM >32GB and GPU acceleration (64GB+ recommended)."
    )?;
    writeln!(out, "3. Skip for now")?;
    if !cloud_logged_in {
        writeln!(
            out,
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
        )?;
    }

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading semantic summary setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "cloud" | "bitloops" => return Ok(SummarySetupSelection::Cloud),
            "2" | "local" | "ollama" => return Ok(SummarySetupSelection::Local),
            "3" | "skip" | "later" => return Ok(SummarySetupSelection::Skip),
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}
