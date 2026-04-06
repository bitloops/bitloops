use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::host::checkpoints::transcript::metadata::{
    SessionMetadataBundle, build_context_markdown, build_session_metadata_bundle,
    extract_prompts_from_transcript_bytes, extract_summary_from_transcript_bytes,
};
use crate::utils::paths;
use crate::utils::paths::LEGACY_BITLOOPS_METADATA_DIR;

use super::blob_keys::{legacy_id_from_path, parse_incremental_sequence_from_name};
use super::types::{
    RepoSqliteRuntimeStore, RuntimeMetadataBlobType, SessionMetadataSnapshot,
    TaskCheckpointArtefact,
};

impl RepoSqliteRuntimeStore {
    pub(crate) fn import_legacy_checkpoint_metadata_if_needed(&self) -> Result<()> {
        let legacy_root = self.repo_root.join(LEGACY_BITLOOPS_METADATA_DIR);
        if !legacy_root.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(&legacy_root)
            .with_context(|| format!("reading legacy metadata root {}", legacy_root.display()))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!("[bitloops] Warning: failed reading legacy metadata entry: {err}");
                    continue;
                }
            };
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let session_id = entry.file_name().to_string_lossy().to_string();
            let session_dir = entry.path();
            let removable = self.import_legacy_session_metadata_dir(&session_id, &session_dir);
            match removable {
                Ok(true) => {
                    let _ = fs::remove_dir_all(&session_dir);
                }
                Ok(false) => {}
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed importing legacy metadata for session `{session_id}`: {err:#}"
                    );
                }
            }
        }

        if fs::read_dir(&legacy_root)
            .ok()
            .and_then(|mut entries| entries.next())
            .is_none()
        {
            let _ = fs::remove_dir_all(&legacy_root);
        }

        Ok(())
    }

    fn import_legacy_session_metadata_dir(
        &self,
        session_id: &str,
        session_dir: &Path,
    ) -> Result<bool> {
        let mut removable = true;
        let transcript_path = session_dir.join(paths::TRANSCRIPT_FILE_NAME);
        let prompt_path = session_dir.join(paths::PROMPT_FILE_NAME);
        let summary_path = session_dir.join(paths::SUMMARY_FILE_NAME);
        let context_path = session_dir.join(paths::CONTEXT_FILE_NAME);

        if transcript_path.exists()
            || prompt_path.exists()
            || summary_path.exists()
            || context_path.exists()
        {
            let transcript = fs::read(&transcript_path).unwrap_or_default();
            let prompt_text = fs::read_to_string(&prompt_path).unwrap_or_default();
            let summary_text = fs::read_to_string(&summary_path).unwrap_or_default();
            let context = fs::read(&context_path).unwrap_or_default();
            let prompts_from_prompt_file = prompt_text
                .split("\n\n---\n\n")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let derived_prompts = if !prompts_from_prompt_file.is_empty() {
                prompts_from_prompt_file
            } else {
                extract_prompts_from_transcript_bytes(&transcript)
            };
            let commit_message = derived_prompts.last().cloned().unwrap_or_default();
            let mut bundle = if !transcript.is_empty() {
                build_session_metadata_bundle(session_id, &commit_message, &transcript)?
            } else {
                SessionMetadataBundle::default()
            };
            if !derived_prompts.is_empty() {
                bundle.prompts = derived_prompts;
            }
            if !summary_text.trim().is_empty() {
                bundle.summary = summary_text;
            } else if bundle.summary.trim().is_empty() {
                bundle.summary = extract_summary_from_transcript_bytes(&transcript);
            }
            if !context.is_empty() {
                bundle.context = context;
            } else if bundle.context.is_empty() {
                bundle.context = build_context_markdown(
                    session_id,
                    &commit_message,
                    &bundle.prompts,
                    &bundle.summary,
                )
                .into_bytes();
            }
            if bundle.transcript.is_empty() {
                bundle.transcript = transcript;
            }

            if !bundle.transcript.is_empty()
                || !bundle.prompts.is_empty()
                || !bundle.summary.trim().is_empty()
                || !bundle.context.is_empty()
            {
                let mut snapshot = SessionMetadataSnapshot::new(session_id.to_string(), bundle);
                snapshot.snapshot_id = legacy_id_from_path(session_dir);
                snapshot.transcript_path = transcript_path.to_string_lossy().to_string();
                self.save_session_metadata_snapshot(&snapshot)?;
            }
        }

        let tasks_dir = session_dir.join("tasks");
        if tasks_dir.exists() {
            for task_entry in fs::read_dir(&tasks_dir)
                .with_context(|| format!("reading legacy tasks dir {}", tasks_dir.display()))?
            {
                let task_entry = match task_entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!("[bitloops] Warning: failed reading legacy task entry: {err}");
                        removable = false;
                        continue;
                    }
                };
                let Ok(task_file_type) = task_entry.file_type() else {
                    removable = false;
                    continue;
                };
                if !task_file_type.is_dir() {
                    removable = false;
                    continue;
                }
                if !self.import_legacy_task_metadata_dir(
                    session_id,
                    &task_entry.file_name().to_string_lossy(),
                    &task_entry.path(),
                )? {
                    removable = false;
                }
            }
        }

        Ok(removable)
    }

    fn import_legacy_task_metadata_dir(
        &self,
        session_id: &str,
        tool_use_id: &str,
        task_dir: &Path,
    ) -> Result<bool> {
        let mut removable = true;
        for entry in fs::read_dir(task_dir)
            .with_context(|| format!("reading legacy task metadata dir {}", task_dir.display()))?
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed reading legacy task metadata file: {err}"
                    );
                    removable = false;
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                removable = false;
                continue;
            };
            if file_type.is_dir() {
                if name == "checkpoints" {
                    for checkpoint_entry in fs::read_dir(&path).with_context(|| {
                        format!("reading legacy incremental checkpoints {}", path.display())
                    })? {
                        let checkpoint_entry = match checkpoint_entry {
                            Ok(entry) => entry,
                            Err(err) => {
                                eprintln!(
                                    "[bitloops] Warning: failed reading legacy incremental checkpoint: {err}"
                                );
                                removable = false;
                                continue;
                            }
                        };
                        let checkpoint_path = checkpoint_entry.path();
                        let checkpoint_name =
                            checkpoint_entry.file_name().to_string_lossy().to_string();
                        if checkpoint_path.extension().and_then(|ext| ext.to_str()) != Some("json")
                        {
                            removable = false;
                            continue;
                        }
                        let payload = match fs::read(&checkpoint_path) {
                            Ok(payload) => payload,
                            Err(err) => {
                                eprintln!(
                                    "[bitloops] Warning: failed reading legacy incremental checkpoint {}: {err}",
                                    checkpoint_path.display()
                                );
                                removable = false;
                                continue;
                            }
                        };
                        let mut artefact = TaskCheckpointArtefact::new(
                            session_id.to_string(),
                            tool_use_id.to_string(),
                            RuntimeMetadataBlobType::IncrementalCheckpoint,
                            payload,
                        );
                        artefact.artefact_id = legacy_id_from_path(&checkpoint_path);
                        artefact.incremental_sequence =
                            parse_incremental_sequence_from_name(&checkpoint_name);
                        artefact.is_incremental = true;
                        self.save_task_checkpoint_artefact(&artefact)?;
                    }
                } else {
                    removable = false;
                }
                continue;
            }

            let kind = if name == paths::CHECKPOINT_FILE_NAME {
                Some(RuntimeMetadataBlobType::TaskCheckpoint)
            } else if name == paths::PROMPT_FILE_NAME {
                Some(RuntimeMetadataBlobType::Prompt)
            } else if name.starts_with("agent-") && name.ends_with(".jsonl") {
                Some(RuntimeMetadataBlobType::SubagentTranscript)
            } else {
                None
            };
            let Some(kind) = kind else {
                removable = false;
                continue;
            };

            let payload = match fs::read(&path) {
                Ok(payload) => payload,
                Err(err) => {
                    eprintln!(
                        "[bitloops] Warning: failed reading legacy task metadata {}: {err}",
                        path.display()
                    );
                    removable = false;
                    continue;
                }
            };
            let mut artefact = TaskCheckpointArtefact::new(
                session_id.to_string(),
                tool_use_id.to_string(),
                kind,
                payload,
            );
            artefact.artefact_id = legacy_id_from_path(&path);
            if kind == RuntimeMetadataBlobType::SubagentTranscript {
                artefact.agent_id = name
                    .trim_start_matches("agent-")
                    .trim_end_matches(".jsonl")
                    .to_string();
            }
            self.save_task_checkpoint_artefact(&artefact)?;
        }

        Ok(removable)
    }
}
