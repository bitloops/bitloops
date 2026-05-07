use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};

use super::types::{ContextGuidanceSetupSelection, SummarySetupSelection};

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

pub(crate) fn prompt_context_guidance_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
    default_to_local_when_noninteractive: bool,
    cloud_logged_in: bool,
) -> Result<ContextGuidanceSetupSelection> {
    if !interactive {
        return Ok(if cloud_logged_in {
            ContextGuidanceSetupSelection::Cloud
        } else if default_to_local_when_noninteractive {
            ContextGuidanceSetupSelection::Local
        } else {
            ContextGuidanceSetupSelection::Skip
        });
    }

    if can_use_terminal_picker() {
        return prompt_context_guidance_setup_selection_with_picker(out);
    }

    prompt_context_guidance_setup_selection_with_text_input(out, input)
}

fn prompt_summary_setup_selection_with_picker(
    out: &mut dyn Write,
    _cloud_logged_in: bool,
) -> Result<SummarySetupSelection> {
    let options = vec![
        SingleSelectOption::new("Skip for now (recommended)", Vec::new()),
        SingleSelectOption::new(
            "Bitloops Cloud (limited availability)",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local (Ollama)",
            vec!["Runs locally (32GB+ RAM, GPU strongly recommended).".to_string()],
        ),
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
        0 => SummarySetupSelection::Skip,
        1 => SummarySetupSelection::Cloud,
        2 => SummarySetupSelection::Local,
        _ => unreachable!("terminal picker returned invalid summary selection"),
    })
}

fn prompt_context_guidance_setup_selection_with_picker(
    out: &mut dyn Write,
) -> Result<ContextGuidanceSetupSelection> {
    let options = vec![
        SingleSelectOption::new("Skip for now (recommended)", Vec::new()),
        SingleSelectOption::new(
            "Bitloops Cloud (limited availability)",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local (Ollama)",
            vec!["Runs locally (32GB+ RAM, GPU strongly recommended).".to_string()],
        ),
    ];
    let intro = vec![
        "Context guidance distills captured sessions and linked knowledge".to_string(),
        "into repo-specific guidance facts.".to_string(),
    ];

    writeln!(out)?;
    let selection =
        prompt_single_select(out, "Configure context guidance", &intro, &options, 0, &[])?;

    Ok(match selection {
        0 => ContextGuidanceSetupSelection::Skip,
        1 => ContextGuidanceSetupSelection::Cloud,
        2 => ContextGuidanceSetupSelection::Local,
        _ => unreachable!("terminal picker returned invalid context guidance selection"),
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
    writeln!(out, "1. Skip for now (recommended)")?;
    writeln!(out, "2. Bitloops Cloud (limited availability)")?;
    writeln!(out, "   Fast setup. No local compute required.")?;
    writeln!(out, "3. Local (Ollama)")?;
    writeln!(
        out,
        "   Runs locally (32GB+ RAM, GPU strongly recommended)."
    )?;

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading semantic summary setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "skip" | "later" => return Ok(SummarySetupSelection::Skip),
            "2" | "cloud" | "bitloops" => return Ok(SummarySetupSelection::Cloud),
            "3" | "local" | "ollama" => return Ok(SummarySetupSelection::Local),
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}

fn prompt_context_guidance_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<ContextGuidanceSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Configure context guidance")?;
    writeln!(out)?;
    writeln!(
        out,
        "Context guidance distills captured sessions and linked knowledge"
    )?;
    writeln!(out, "into repo-specific guidance facts.")?;
    writeln!(out)?;
    writeln!(out, "1. Skip for now (recommended)")?;
    writeln!(out, "2. Bitloops Cloud (limited availability)")?;
    writeln!(out, "   Fast setup. No local compute required.")?;
    writeln!(out, "3. Local (Ollama)")?;
    writeln!(
        out,
        "   Runs locally (32GB+ RAM, GPU strongly recommended)."
    )?;

    loop {
        writeln!(out, "Select an option [1/2/3]")?;
        write!(out, "> ")?;
        out.flush()?;

        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading context guidance setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "skip" | "later" => return Ok(ContextGuidanceSetupSelection::Skip),
            "2" | "cloud" | "bitloops" => return Ok(ContextGuidanceSetupSelection::Cloud),
            "3" | "local" | "ollama" => return Ok(ContextGuidanceSetupSelection::Local),
            _ => writeln!(out, "Please choose 1, 2, or 3.")?,
        }
    }
}
