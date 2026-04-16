use anyhow::Result;
use std::path::Path;

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn rotate_daemon_log_file(_log_path: &Path, _retention: usize) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn log_path(root: &Path) -> PathBuf {
        root.join("daemon.log")
    }

    fn archive_path(root: &Path, index: usize) -> PathBuf {
        root.join(format!("daemon.log.{index}"))
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write file");
    }

    fn read_file(path: &Path) -> String {
        fs::read_to_string(path).expect("read file")
    }

    #[test]
    fn daemon_log_sink_rotates_current_file_into_archive() {
        let temp = TempDir::new().expect("temp dir");
        let current_log = log_path(temp.path());
        let archive_1 = archive_path(temp.path(), 1);
        let archive_2 = archive_path(temp.path(), 2);

        write_file(&current_log, "current\n");
        write_file(&archive_1, "archive-1\n");
        write_file(&archive_2, "archive-2\n");

        rotate_daemon_log_file(&current_log, 5).expect("rotate daemon log");

        assert_eq!(read_file(&archive_1), "current\n");
        assert_eq!(read_file(&archive_2), "archive-1\n");
        assert_eq!(read_file(&archive_path(temp.path(), 3)), "archive-2\n");
    }

    #[test]
    fn daemon_log_sink_recreates_active_file_after_rotation() {
        let temp = TempDir::new().expect("temp dir");
        let current_log = log_path(temp.path());

        write_file(&current_log, "current\n");

        rotate_daemon_log_file(&current_log, 5).expect("rotate daemon log");

        assert!(current_log.exists());
        assert_eq!(read_file(&current_log), "");
    }

    #[test]
    fn daemon_log_sink_drops_oldest_archive_after_retention_limit() {
        let temp = TempDir::new().expect("temp dir");
        let current_log = log_path(temp.path());

        write_file(&current_log, "current\n");
        for index in 1..=5 {
            write_file(&archive_path(temp.path(), index), &format!("archive-{index}\n"));
        }

        rotate_daemon_log_file(&current_log, 5).expect("rotate daemon log");

        assert_eq!(read_file(&archive_path(temp.path(), 1)), "current\n");
        assert_eq!(read_file(&archive_path(temp.path(), 2)), "archive-1\n");
        assert_eq!(read_file(&archive_path(temp.path(), 3)), "archive-2\n");
        assert_eq!(read_file(&archive_path(temp.path(), 4)), "archive-3\n");
        assert_eq!(read_file(&archive_path(temp.path(), 5)), "archive-4\n");
        assert!(!archive_path(temp.path(), 6).exists());
    }
}
