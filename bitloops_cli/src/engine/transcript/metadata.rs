use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TaskCheckpoint {
    pub session_id: String,
    pub tool_use_id: String,
    pub checkpoint_uuid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
}

pub fn task_metadata_dir(session_metadata_dir: &str, tool_use_id: &str) -> String {
    PathBuf::from(session_metadata_dir)
        .join("tasks")
        .join(tool_use_id)
        .to_string_lossy()
        .to_string()
}

pub fn write_task_checkpoint(task_metadata_dir: &str, checkpoint: &TaskCheckpoint) -> Result<()> {
    fs::create_dir_all(task_metadata_dir).with_context(|| {
        format!("failed to create task metadata directory: {task_metadata_dir}")
    })?;

    let mut data =
        serde_json::to_string_pretty(checkpoint).context("failed to marshal checkpoint")?;
    data.push('\n');

    let checkpoint_file =
        PathBuf::from(task_metadata_dir).join(crate::utils::paths::CHECKPOINT_FILE_NAME);
    fs::write(&checkpoint_file, data).with_context(|| {
        format!(
            "failed to write checkpoint file: {}",
            checkpoint_file.to_string_lossy()
        )
    })?;

    Ok(())
}

pub fn read_task_checkpoint(task_metadata_dir: &str) -> Result<TaskCheckpoint> {
    let checkpoint_file =
        PathBuf::from(task_metadata_dir).join(crate::utils::paths::CHECKPOINT_FILE_NAME);
    let data = fs::read(&checkpoint_file).with_context(|| {
        format!(
            "failed to read checkpoint file: {}",
            checkpoint_file.to_string_lossy()
        )
    })?;

    serde_json::from_slice(&data).context("failed to unmarshal checkpoint")
}

pub fn write_task_prompt(task_metadata_dir: &str, prompt: &str) -> Result<()> {
    let prompt_file = PathBuf::from(task_metadata_dir).join(crate::utils::paths::PROMPT_FILE_NAME);
    fs::write(&prompt_file, prompt).with_context(|| {
        format!(
            "failed to write prompt file: {}",
            prompt_file.to_string_lossy()
        )
    })?;
    Ok(())
}

pub fn copy_agent_transcript(
    src_transcript: &str,
    task_metadata_dir: &str,
    agent_id: &str,
) -> Result<()> {
    let src = PathBuf::from(src_transcript);
    if !src.exists() {
        return Ok(());
    }

    let dst = PathBuf::from(task_metadata_dir).join(format!("agent-{agent_id}.jsonl"));
    fs::copy(&src, &dst).with_context(|| {
        format!(
            "failed to copy transcript from {} to {}",
            src.to_string_lossy(),
            dst.to_string_lossy()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{
        TaskCheckpoint, copy_agent_transcript, read_task_checkpoint, task_metadata_dir,
        write_task_checkpoint, write_task_prompt,
    };

    #[test]
    fn test_task_metadata_dir() {
        let session_metadata_dir = ".bitloops/metadata/2025-01-28-abc123";
        let tool_use_id = "toolu_xyz789";

        let expected = ".bitloops/metadata/2025-01-28-abc123/tasks/toolu_xyz789";
        let got = task_metadata_dir(session_metadata_dir, tool_use_id);

        assert_eq!(got, expected);
    }

    #[test]
    fn test_task_checkpoint() {
        let tmp_dir = tempdir().expect("failed to create temp dir");
        let task_metadata_dir = tmp_dir.path().join("tasks").join("toolu_test123");
        let task_metadata_dir = task_metadata_dir.to_string_lossy().to_string();

        let checkpoint = TaskCheckpoint {
            session_id: "session-abc".to_string(),
            tool_use_id: "toolu_test123".to_string(),
            checkpoint_uuid: "uuid-checkpoint-123".to_string(),
            agent_id: "agent_subagent_001".to_string(),
        };

        write_task_checkpoint(&task_metadata_dir, &checkpoint).expect("write should succeed");

        let task_dir = PathBuf::from(&task_metadata_dir);
        assert!(
            task_dir.exists(),
            "task metadata directory should be created"
        );

        let checkpoint_file = task_dir.join(crate::utils::paths::CHECKPOINT_FILE_NAME);
        let data = fs::read(&checkpoint_file).expect("failed reading checkpoint file");
        let loaded: TaskCheckpoint =
            serde_json::from_slice(&data).expect("failed unmarshaling checkpoint");

        assert_eq!(loaded.session_id, checkpoint.session_id);
        assert_eq!(loaded.tool_use_id, checkpoint.tool_use_id);
        assert_eq!(loaded.checkpoint_uuid, checkpoint.checkpoint_uuid);
        assert_eq!(loaded.agent_id, checkpoint.agent_id);

        let read_checkpoint =
            read_task_checkpoint(&task_metadata_dir).expect("read should succeed");
        assert_eq!(read_checkpoint.session_id, checkpoint.session_id);
    }

    #[test]
    fn test_write_task_prompt() {
        let tmp_dir = tempdir().expect("failed to create temp dir");
        let task_metadata_dir = tmp_dir.path().join("tasks").join("toolu_test");
        fs::create_dir_all(&task_metadata_dir).expect("failed to create metadata dir");
        let task_metadata_dir = task_metadata_dir.to_string_lossy().to_string();

        let prompt = "Please implement the feature described in task-01.md";
        write_task_prompt(&task_metadata_dir, prompt).expect("write prompt should succeed");

        let prompt_file =
            PathBuf::from(&task_metadata_dir).join(crate::utils::paths::PROMPT_FILE_NAME);
        let data = fs::read_to_string(prompt_file).expect("failed to read prompt file");
        assert_eq!(data, prompt);
    }

    #[test]
    fn test_copy_agent_transcript() {
        let tmp_dir = tempdir().expect("failed to create temp dir");
        let src_dir = tmp_dir.path().join("source");
        fs::create_dir_all(&src_dir).expect("failed to create source dir");

        let src_transcript = src_dir.join("agent-test_agent.jsonl");
        let transcript_content = "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"test\"}}\n{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[]}}";
        fs::write(&src_transcript, transcript_content).expect("failed writing source transcript");

        let dst_dir = tmp_dir.path().join("dest").join("tasks").join("toolu_test");
        fs::create_dir_all(&dst_dir).expect("failed to create destination dir");

        copy_agent_transcript(
            &src_transcript.to_string_lossy(),
            &dst_dir.to_string_lossy(),
            "test_agent",
        )
        .expect("copy should succeed");

        let dst_transcript = dst_dir.join("agent-test_agent.jsonl");
        let data = fs::read_to_string(dst_transcript).expect("failed reading copied transcript");
        assert_eq!(data, transcript_content);
    }

    #[test]
    fn test_copy_agent_transcript_source_not_exists() {
        let tmp_dir = tempdir().expect("failed to create temp dir");
        let dst_dir = tmp_dir.path().join("dest");
        fs::create_dir_all(&dst_dir).expect("failed creating destination dir");

        copy_agent_transcript(
            "/nonexistent/path.jsonl",
            &dst_dir.to_string_lossy(),
            "test",
        )
        .expect("copy should no-op when source does not exist");
    }
}
