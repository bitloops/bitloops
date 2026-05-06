use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use crate::cli::terminal_picker::{
    SingleSelectOption, can_use_terminal_picker, prompt_single_select,
};

use super::types::{
    BitloopsInferenceSetupSelection, ContextGuidanceSetupSelection, SummarySetupSelection,
};

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

pub(crate) fn prompt_bitloops_inference_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
    cloud_logged_in: bool,
) -> Result<BitloopsInferenceSetupSelection> {
    if !interactive {
        return Ok(BitloopsInferenceSetupSelection::Skip);
    }

    if can_use_terminal_picker() {
        return prompt_bitloops_inference_setup_selection_with_picker(out, cloud_logged_in);
    }

    prompt_bitloops_inference_setup_selection_with_text_input(out, input, cloud_logged_in)
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

fn prompt_bitloops_inference_setup_selection_with_picker(
    out: &mut dyn Write,
    cloud_logged_in: bool,
) -> Result<BitloopsInferenceSetupSelection> {
    let options = vec![
        SingleSelectOption::new(
            "Bitloops Cloud",
            vec!["Fast setup. No local compute required.".to_string()],
        ),
        SingleSelectOption::new(
            "Local (Ollama)",
            vec!["Runs locally (32GB+ RAM, GPU strongly recommended).".to_string()],
        ),
        SingleSelectOption::new("Skip for now", Vec::new()),
    ];
    let intro = vec![
        "Bitloops inference powers semantic summaries, context guidance,".to_string(),
        "architecture fact synthesis, and architecture role adjudication.".to_string(),
    ];

    writeln!(out)?;
    let selection = prompt_single_select(
        out,
        "Enable Bitloops inference",
        &intro,
        &options,
        if cloud_logged_in { 0 } else { 2 },
        &[],
    )?;

    Ok(match selection {
        0 => BitloopsInferenceSetupSelection::Cloud,
        1 => BitloopsInferenceSetupSelection::Local,
        _ => BitloopsInferenceSetupSelection::Skip,
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

fn prompt_bitloops_inference_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    cloud_logged_in: bool,
) -> Result<BitloopsInferenceSetupSelection> {
    writeln!(out)?;
    writeln!(out, "Enable Bitloops inference?")?;
    writeln!(
        out,
        "Bitloops inference powers semantic summaries, context guidance, architecture fact synthesis, and architecture role adjudication."
    )?;
    writeln!(out, "  1. Bitloops Cloud")?;
    writeln!(out, "  2. Local (Ollama)")?;
    writeln!(out, "  3. Skip for now")?;
    if cloud_logged_in {
        writeln!(out, "Press Enter to use Bitloops Cloud.")?;
    } else {
        writeln!(out, "Press Enter to skip for now.")?;
    }

    loop {
        write!(out, "> ")?;
        out.flush()?;
        let mut line = String::new();
        input
            .read_line(&mut line)
            .context("reading Bitloops inference setup selection")?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" if cloud_logged_in => return Ok(BitloopsInferenceSetupSelection::Cloud),
            "" => return Ok(BitloopsInferenceSetupSelection::Skip),
            "1" | "cloud" | "bitloops" => return Ok(BitloopsInferenceSetupSelection::Cloud),
            "2" | "local" | "ollama" => return Ok(BitloopsInferenceSetupSelection::Local),
            "3" | "skip" | "later" => return Ok(BitloopsInferenceSetupSelection::Skip),
            _ => writeln!(out, "Please choose `1`, `2`, or `3`.")?,
        }
    }
}
