use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::inference::managed::{
    ManagedInferenceInstallPhase, ManagedInferenceInstallProgress, install_or_bootstrap_inference,
    install_or_bootstrap_inference_with_progress,
};

use super::constants::{
    DEFAULT_OLLAMA_BASE_URL, DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV,
    DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL, DEFAULT_PLATFORM_SUMMARY_MODEL,
};
use super::ollama::{
    auto_configured_ollama_model_name, auto_configured_summary_model_name,
    probe_ollama_availability, select_ollama_model, select_ollama_model_for_label,
};
use super::profiles::{
    write_context_guidance_profile, write_platform_context_guidance_profile,
    write_platform_summary_profile, write_summary_profile,
};
#[cfg(test)]
use super::profiles::{
    write_local_bitloops_inference_profiles, write_platform_bitloops_inference_profiles,
};
use super::types::{
    ContextGuidanceSetupOutcome, OllamaAvailability, PreparedSummarySetupAction,
    PreparedSummarySetupPlan, SummarySetupExecutionResult, SummarySetupOutcome, SummarySetupPhase,
    SummarySetupProgress,
};

enum PreparedContextGuidanceSetupAction {
    InstallRuntimeOnly {
        message: String,
    },
    InstallRuntimeOnlyPendingProbe {
        message: String,
    },
    ConfigureLocal {
        model_name: String,
    },
    ConfigureCloud {
        gateway_url_override: Option<String>,
        api_key_env: String,
    },
}

struct PreparedContextGuidanceSetupPlan {
    action: PreparedContextGuidanceSetupAction,
}

pub(crate) fn configure_local_summary_generation(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<SummarySetupOutcome> {
    let plan = prepare_local_summary_generation_plan(out, input, interactive)?;
    let lines = install_or_bootstrap_inference(repo_root)?;
    for line in lines {
        writeln!(out, "{line}")?;
    }

    let execution = apply_prepared_summary_setup(repo_root, plan)?;
    writeln!(out, "{}", execution.message)?;
    Ok(execution.outcome)
}

pub(crate) fn configure_cloud_summary_generation(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
) -> Result<String> {
    let _ = install_or_bootstrap_inference(repo_root)?;
    let execution = apply_prepared_summary_setup(
        repo_root,
        prepare_cloud_summary_generation_plan(gateway_url_override),
    )?;
    Ok(execution.message)
}

pub(crate) fn configure_local_context_guidance_generation(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<ContextGuidanceSetupOutcome> {
    let plan = prepare_local_context_guidance_generation_plan(out, input, interactive)?;
    let lines = install_or_bootstrap_inference(repo_root)?;
    for line in lines {
        writeln!(out, "{line}")?;
    }

    let execution = apply_prepared_context_guidance_setup(repo_root, plan)?;
    writeln!(out, "{}", execution.1)?;
    Ok(execution.0)
}

pub(crate) fn configure_cloud_context_guidance_generation(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<String> {
    let _ = install_or_bootstrap_inference(repo_root)?;
    let plan = PreparedContextGuidanceSetupPlan {
        action: PreparedContextGuidanceSetupAction::ConfigureCloud {
            gateway_url_override: gateway_url_override.map(str::to_string),
            api_key_env: api_key_env
                .map(str::to_string)
                .unwrap_or_else(|| DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV.to_string()),
        },
    };
    let execution = apply_prepared_context_guidance_setup(repo_root, plan)?;
    Ok(execution.1)
}

#[cfg(test)]
pub(crate) fn configure_local_bitloops_inference(
    repo_root: &Path,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<SummarySetupOutcome> {
    let plan = prepare_local_bitloops_inference_plan(out, input, interactive)?;
    let lines = install_or_bootstrap_inference(repo_root)?;
    for line in lines {
        writeln!(out, "{line}")?;
    }

    let execution = apply_prepared_bitloops_inference_setup(repo_root, plan)?;
    writeln!(out, "{}", execution.message)?;
    Ok(execution.outcome)
}

#[cfg(test)]
pub(crate) fn configure_cloud_bitloops_inference(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
    api_key_env: Option<&str>,
) -> Result<String> {
    let _ = install_or_bootstrap_inference(repo_root)?;
    let execution = apply_prepared_bitloops_inference_setup(
        repo_root,
        prepare_cloud_bitloops_inference_plan(gateway_url_override, api_key_env),
    )?;
    Ok(execution.message)
}

pub(crate) fn prepare_cloud_summary_generation_plan(
    gateway_url_override: Option<&str>,
) -> PreparedSummarySetupPlan {
    PreparedSummarySetupPlan {
        action: PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override: gateway_url_override.map(str::to_string),
            api_key_env: None,
        },
    }
}

#[cfg(test)]
pub(crate) fn prepare_cloud_bitloops_inference_plan(
    gateway_url_override: Option<&str>,
    api_key_env: Option<&str>,
) -> PreparedSummarySetupPlan {
    PreparedSummarySetupPlan {
        action: PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override: gateway_url_override.map(str::to_string),
            api_key_env: Some(
                api_key_env
                    .map(str::to_string)
                    .unwrap_or_else(|| DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV.to_string()),
            ),
        },
    }
}

#[cfg(test)]
pub(crate) fn prepare_local_bitloops_inference_plan(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<PreparedSummarySetupPlan> {
    let mut availability = probe_ollama_availability()?;
    loop {
        match availability {
            OllamaAvailability::MissingCli => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnly {
                        message: "Ollama was not found on PATH; installed `bitloops-inference` but skipped Bitloops inference setup.".to_string(),
                    },
                });
            }
            OllamaAvailability::NotRunning if interactive => {
                writeln!(
                    out,
                    "Ollama is installed but not responding at {DEFAULT_OLLAMA_BASE_URL}."
                )?;
                writeln!(
                    out,
                    "Retry Bitloops inference setup or skip it for now? (r/S)"
                )?;
                write!(out, "> ")?;
                out.flush()?;
                let mut line = String::new();
                input
                    .read_line(&mut line)
                    .context("reading Ollama retry prompt response")?;
                match line.trim().to_ascii_lowercase().as_str() {
                    "r" | "retry" => {
                        availability = probe_ollama_availability()?;
                        continue;
                    }
                    "" | "s" | "skip" => {
                        return Ok(PreparedSummarySetupPlan {
                            action: PreparedSummarySetupAction::InstallRuntimeOnly {
                                message: "Installed `bitloops-inference`; skipped Bitloops inference setup because Ollama is not running.".to_string(),
                            },
                        });
                    }
                    _ => {
                        writeln!(out, "Please answer `r` to retry or `s` to skip.")?;
                        continue;
                    }
                }
            }
            OllamaAvailability::NotRunning => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe {
                        message: "Installed `bitloops-inference`; skipped Bitloops inference setup because Ollama is not running.".to_string(),
                    },
                });
            }
            OllamaAvailability::Running { ref models } => {
                let model_name = select_ollama_model_for_label(
                    models,
                    "Bitloops inference",
                    out,
                    input,
                    interactive,
                )?;
                let Some(model_name) = model_name else {
                    return Ok(PreparedSummarySetupPlan {
                        action: PreparedSummarySetupAction::InstallRuntimeOnly {
                            message: "Installed `bitloops-inference`; skipped Bitloops inference profile setup.".to_string(),
                        },
                    });
                };
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::ConfigureLocal { model_name },
                });
            }
        }
    }
}

pub(crate) fn prepare_local_summary_generation_plan(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<PreparedSummarySetupPlan> {
    let mut availability = probe_ollama_availability()?;
    loop {
        match availability {
            OllamaAvailability::MissingCli => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnly {
                        message: "Ollama was not found on PATH; installed `bitloops-inference` but skipped semantic summary setup.".to_string(),
                    },
                });
            }
            OllamaAvailability::NotRunning if interactive => {
                writeln!(
                    out,
                    "Ollama is installed but not responding at {DEFAULT_OLLAMA_BASE_URL}."
                )?;
                writeln!(out, "Retry summary setup or skip it for now? (r/S)")?;
                write!(out, "> ")?;
                out.flush()?;
                let mut line = String::new();
                input
                    .read_line(&mut line)
                    .context("reading Ollama retry prompt response")?;
                match line.trim().to_ascii_lowercase().as_str() {
                    "r" | "retry" => {
                        availability = probe_ollama_availability()?;
                        continue;
                    }
                    "" | "s" | "skip" => {
                        return Ok(PreparedSummarySetupPlan {
                            action: PreparedSummarySetupAction::InstallRuntimeOnly {
                                message: "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running.".to_string(),
                            },
                        });
                    }
                    _ => {
                        writeln!(out, "Please answer `r` to retry or `s` to skip.")?;
                        continue;
                    }
                }
            }
            OllamaAvailability::NotRunning => {
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe {
                        message: "Installed `bitloops-inference`; skipped semantic summary setup because Ollama is not running.".to_string(),
                    },
                });
            }
            OllamaAvailability::Running { ref models } => {
                let model_name = select_ollama_model(models, out, input, interactive)?;
                let Some(model_name) = model_name else {
                    return Ok(PreparedSummarySetupPlan {
                        action: PreparedSummarySetupAction::InstallRuntimeOnly {
                            message: "Installed `bitloops-inference`; skipped semantic summary profile setup.".to_string(),
                        },
                    });
                };
                return Ok(PreparedSummarySetupPlan {
                    action: PreparedSummarySetupAction::ConfigureLocal { model_name },
                });
            }
        }
    }
}

fn prepare_local_context_guidance_generation_plan(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    interactive: bool,
) -> Result<PreparedContextGuidanceSetupPlan> {
    let mut availability = probe_ollama_availability()?;
    loop {
        match availability {
            OllamaAvailability::MissingCli => {
                return Ok(PreparedContextGuidanceSetupPlan {
                    action: PreparedContextGuidanceSetupAction::InstallRuntimeOnly {
                        message: "Ollama was not found on PATH; installed `bitloops-inference` but skipped context guidance setup.".to_string(),
                    },
                });
            }
            OllamaAvailability::NotRunning if interactive => {
                writeln!(
                    out,
                    "Ollama is installed but not responding at {DEFAULT_OLLAMA_BASE_URL}."
                )?;
                writeln!(
                    out,
                    "Retry context guidance setup or skip it for now? (r/S)"
                )?;
                write!(out, "> ")?;
                out.flush()?;
                let mut line = String::new();
                input
                    .read_line(&mut line)
                    .context("reading Ollama retry prompt response")?;
                match line.trim().to_ascii_lowercase().as_str() {
                    "r" | "retry" => {
                        availability = probe_ollama_availability()?;
                        continue;
                    }
                    "" | "s" | "skip" => {
                        return Ok(PreparedContextGuidanceSetupPlan {
                            action: PreparedContextGuidanceSetupAction::InstallRuntimeOnly {
                                message: "Installed `bitloops-inference`; skipped context guidance setup because Ollama is not running.".to_string(),
                            },
                        });
                    }
                    _ => {
                        writeln!(out, "Please answer `r` to retry or `s` to skip.")?;
                        continue;
                    }
                }
            }
            OllamaAvailability::NotRunning => {
                return Ok(PreparedContextGuidanceSetupPlan {
                    action: PreparedContextGuidanceSetupAction::InstallRuntimeOnlyPendingProbe {
                        message: "Installed `bitloops-inference`; skipped context guidance setup because Ollama is not running.".to_string(),
                    },
                });
            }
            OllamaAvailability::Running { ref models } => {
                let model_name = select_ollama_model_for_label(
                    models,
                    "context guidance",
                    out,
                    input,
                    interactive,
                )?;
                let Some(model_name) = model_name else {
                    return Ok(PreparedContextGuidanceSetupPlan {
                        action: PreparedContextGuidanceSetupAction::InstallRuntimeOnly {
                            message: "Installed `bitloops-inference`; skipped context guidance profile setup.".to_string(),
                        },
                    });
                };
                return Ok(PreparedContextGuidanceSetupPlan {
                    action: PreparedContextGuidanceSetupAction::ConfigureLocal { model_name },
                });
            }
        }
    }
}

pub(crate) fn execute_prepared_summary_setup_with_progress<R>(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
    mut report: R,
) -> Result<SummarySetupExecutionResult>
where
    R: FnMut(SummarySetupProgress) -> Result<()>,
{
    install_or_bootstrap_inference_with_progress(repo_root, |progress| {
        report(summary_setup_progress_from_managed(progress))
    })?;
    apply_prepared_summary_setup_with_progress(repo_root, plan, &mut report)
}

fn apply_prepared_context_guidance_setup(
    repo_root: &Path,
    plan: PreparedContextGuidanceSetupPlan,
) -> Result<(ContextGuidanceSetupOutcome, String)> {
    match plan.action {
        PreparedContextGuidanceSetupAction::InstallRuntimeOnly { message } => {
            Ok((ContextGuidanceSetupOutcome::InstalledRuntimeOnly, message))
        }
        PreparedContextGuidanceSetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            if let Some(model_name) = auto_configured_ollama_model_name()? {
                write_context_guidance_profile(repo_root, &model_name)?;
                return Ok((
                    ContextGuidanceSetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    format!("Configured context guidance to use Ollama model `{model_name}`."),
                ));
            }

            Ok((ContextGuidanceSetupOutcome::InstalledRuntimeOnly, message))
        }
        PreparedContextGuidanceSetupAction::ConfigureLocal { model_name } => {
            write_context_guidance_profile(repo_root, &model_name)?;
            Ok((
                ContextGuidanceSetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                format!("Configured context guidance to use Ollama model `{model_name}`."),
            ))
        }
        PreparedContextGuidanceSetupAction::ConfigureCloud {
            gateway_url_override,
            api_key_env,
        } => {
            write_platform_context_guidance_profile(
                repo_root,
                gateway_url_override.as_deref(),
                &api_key_env,
            )?;
            Ok((
                ContextGuidanceSetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL.to_string(),
                },
                "Configured context guidance to use Bitloops cloud context guidance.".to_string(),
            ))
        }
    }
}

#[cfg(test)]
fn apply_prepared_bitloops_inference_setup(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
) -> Result<SummarySetupExecutionResult> {
    match plan.action {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            if let Some(model_name) = auto_configured_ollama_model_name()? {
                write_local_bitloops_inference_profiles(repo_root, &model_name)?;
                return Ok(SummarySetupExecutionResult {
                    outcome: SummarySetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    message: format!(
                        "Configured Bitloops inference to use Ollama model `{model_name}`."
                    ),
                });
            }

            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            write_local_bitloops_inference_profiles(repo_root, &model_name)?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                message: format!(
                    "Configured Bitloops inference to use Ollama model `{model_name}`."
                ),
            })
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
            api_key_env,
        } => {
            let api_key_env = api_key_env
                .as_deref()
                .unwrap_or(DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV);
            write_platform_bitloops_inference_profiles(
                repo_root,
                gateway_url_override.as_deref(),
                api_key_env,
            )?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL.to_string(),
                },
                message: "Configured Bitloops inference to use Bitloops cloud.".to_string(),
            })
        }
    }
}

fn apply_prepared_summary_setup(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
) -> Result<SummarySetupExecutionResult> {
    match plan.action {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            if let Some(model_name) = auto_configured_summary_model_name()? {
                write_summary_profile(repo_root, &model_name)?;
                return Ok(SummarySetupExecutionResult {
                    outcome: SummarySetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    message: format!(
                        "Configured semantic summaries to use Ollama model `{model_name}`."
                    ),
                });
            }

            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            write_summary_profile(repo_root, &model_name)?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                message: format!(
                    "Configured semantic summaries to use Ollama model `{model_name}`."
                ),
            })
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
            api_key_env: _,
        } => {
            write_platform_summary_profile(repo_root, gateway_url_override.as_deref())?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_SUMMARY_MODEL.to_string(),
                },
                message: "Configured semantic summaries to use Bitloops cloud summaries."
                    .to_string(),
            })
        }
    }
}

fn apply_prepared_summary_setup_with_progress<R>(
    repo_root: &Path,
    plan: PreparedSummarySetupPlan,
    report: &mut R,
) -> Result<SummarySetupExecutionResult>
where
    R: FnMut(SummarySetupProgress) -> Result<()>,
{
    match plan.action {
        PreparedSummarySetupAction::InstallRuntimeOnly { message } => {
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe { message } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some("Rechecking Ollama before applying summary profile".to_string()),
                ..Default::default()
            })?;
            if let Some(model_name) = auto_configured_summary_model_name()? {
                report(SummarySetupProgress {
                    phase: SummarySetupPhase::WritingProfile,
                    message: Some(format!("Applying summary profile for `{model_name}`")),
                    ..Default::default()
                })?;
                write_summary_profile(repo_root, &model_name)?;
                return Ok(SummarySetupExecutionResult {
                    outcome: SummarySetupOutcome::Configured {
                        model_name: model_name.clone(),
                    },
                    message: format!(
                        "Configured semantic summaries to use Ollama model `{model_name}`."
                    ),
                });
            }

            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::InstalledRuntimeOnly,
                message,
            })
        }
        PreparedSummarySetupAction::ConfigureLocal { model_name } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some(format!("Applying summary profile for `{model_name}`")),
                ..Default::default()
            })?;
            write_summary_profile(repo_root, &model_name)?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: model_name.clone(),
                },
                message: format!(
                    "Configured semantic summaries to use Ollama model `{model_name}`."
                ),
            })
        }
        PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override,
            api_key_env: _,
        } => {
            report(SummarySetupProgress {
                phase: SummarySetupPhase::WritingProfile,
                message: Some("Applying Bitloops cloud summary profile".to_string()),
                ..Default::default()
            })?;
            write_platform_summary_profile(repo_root, gateway_url_override.as_deref())?;
            Ok(SummarySetupExecutionResult {
                outcome: SummarySetupOutcome::Configured {
                    model_name: DEFAULT_PLATFORM_SUMMARY_MODEL.to_string(),
                },
                message: "Configured semantic summaries to use Bitloops cloud summaries."
                    .to_string(),
            })
        }
    }
}

fn summary_setup_progress_from_managed(
    progress: ManagedInferenceInstallProgress,
) -> SummarySetupProgress {
    SummarySetupProgress {
        phase: match progress.phase {
            ManagedInferenceInstallPhase::Queued => SummarySetupPhase::Queued,
            ManagedInferenceInstallPhase::ResolvingRelease => SummarySetupPhase::ResolvingRelease,
            ManagedInferenceInstallPhase::DownloadingRuntime => {
                SummarySetupPhase::DownloadingRuntime
            }
            ManagedInferenceInstallPhase::ExtractingRuntime => SummarySetupPhase::ExtractingRuntime,
            ManagedInferenceInstallPhase::RewritingRuntime => SummarySetupPhase::RewritingRuntime,
        },
        asset_name: progress.asset_name,
        bytes_downloaded: progress.bytes_downloaded,
        bytes_total: progress.bytes_total,
        version: progress.version,
        message: progress.message,
    }
}
