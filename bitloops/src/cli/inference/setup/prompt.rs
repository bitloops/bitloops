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
    _cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops Cloud (recommended)",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local (Ollama)",
            vec!["Runs locally (32GB+ RAM, GPU strongly recommended).".to_string()],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];
    let intro = vec![
        "Summaries help agents understand your code structure".to_string(),
        "(e.g. file purposes, module responsibilities).".to_string(),
    ];

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "Configure semantic summaries",
        &intro,
        &options,
        0,
        &[],
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
    _cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure semantic summaries")?;
    writeln!(out)?;
    writeln!(out, "Summaries help agents understand your code structure")?;
    writeln!(out, "(e.g. file purposes, module responsibilities).")?;
    writeln!(out)?;
    writeln!(out, "1. Bitloops Cloud (recommended)")?;
    writeln!(out, "   Fast setup. No local compute required.")?;
    writeln!(out, "2. Local (Ollama)")?;
    writeln!(
        out,
        "   Runs locally (32GB+ RAM, GPU strongly recommended)."
    )?;
    writeln!(out, "3. Skip for now")?;

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
