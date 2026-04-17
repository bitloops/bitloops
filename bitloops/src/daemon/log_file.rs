use anyhow::{Context, Result, anyhow};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

pub(crate) const DAEMON_LOG_ROTATION_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const DAEMON_LOG_RETENTION: usize = 5;

pub(crate) fn open_daemon_log_sink(log_path: &Path) -> Result<DaemonLogSink> {
    DaemonLogSink::open(log_path)
}

pub(crate) struct DaemonLogSink {
    log_path: PathBuf,
    file: Option<File>,
}

impl DaemonLogSink {
    fn open(log_path: &Path) -> Result<Self> {
        ensure_parent_directory(log_path)?;
        if active_file_len(log_path)? > DAEMON_LOG_ROTATION_BYTES {
            rotate_daemon_log_file(log_path, DAEMON_LOG_RETENTION)?;
        }

        let file = open_append_file(log_path)?;
        Ok(Self {
            log_path: log_path.to_path_buf(),
            file: Some(file),
        })
    }

    fn rotate_before_append(&mut self, incoming_len: usize) -> io::Result<()> {
        let current_len = active_file_len(&self.log_path).map_err(to_io_error)?;
        let incoming_len = u64::try_from(incoming_len).unwrap_or(u64::MAX);
        let would_exceed_limit =
            current_len > 0 && current_len.saturating_add(incoming_len) > DAEMON_LOG_ROTATION_BYTES;

        if current_len > DAEMON_LOG_ROTATION_BYTES || would_exceed_limit {
            self.rotate()?;
        }
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()?;
        let current_file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("daemon log sink file is not open"))?;
        drop(current_file);

        if let Err(err) =
            rotate_daemon_log_file(&self.log_path, DAEMON_LOG_RETENTION).map_err(to_io_error)
        {
            self.file = Some(open_append_file(&self.log_path).map_err(to_io_error)?);
            return Err(err);
        }

        self.file = Some(open_append_file(&self.log_path).map_err(to_io_error)?);
        Ok(())
    }

    fn append(&mut self, buf: &[u8]) -> io::Result<()> {
        self.rotate_before_append(buf.len())?;
        self.file_mut()?.write_all(buf)
    }

    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("daemon log sink file is not open"))
    }
}

impl Write for DaemonLogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.append(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.append(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn rotate_daemon_log_file(log_path: &Path, retention: usize) -> Result<()> {
    ensure_parent_directory(log_path)?;

    if retention == 0 {
        if log_path.exists() {
            fs::remove_file(log_path).with_context(|| {
                format!(
                    "removing daemon log file before recreating {}",
                    log_path.display()
                )
            })?;
        }
        File::create(log_path)
            .with_context(|| format!("recreating daemon log file {}", log_path.display()))?;
        return Ok(());
    }

    let oldest_archive = archive_path(log_path, retention)?;
    if oldest_archive.exists() {
        fs::remove_file(&oldest_archive).with_context(|| {
            format!(
                "removing oldest daemon log archive {}",
                oldest_archive.display()
            )
        })?;
    }

    for index in (1..retention).rev() {
        let source = archive_path(log_path, index)?;
        if !source.exists() {
            continue;
        }

        let destination = archive_path(log_path, index + 1)?;
        fs::rename(&source, &destination).with_context(|| {
            format!(
                "rotating daemon log archive {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }

    if log_path.exists() {
        let archive_1 = archive_path(log_path, 1)?;
        fs::rename(log_path, &archive_1).with_context(|| {
            format!(
                "rotating active daemon log {} to {}",
                log_path.display(),
                archive_1.display()
            )
        })?;
    }

    File::create(log_path)
        .with_context(|| format!("creating daemon log file {}", log_path.display()))?;
    Ok(())
}

fn ensure_parent_directory(log_path: &Path) -> Result<()> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating daemon log directory {}", parent.display()))?;
    }
    Ok(())
}

fn open_append_file(log_path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("opening daemon log file {}", log_path.display()))
}

fn active_file_len(log_path: &Path) -> Result<u64> {
    match fs::metadata(log_path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(0),
        Err(err) => {
            Err(err).with_context(|| format!("reading daemon log file {}", log_path.display()))
        }
    }
}

fn archive_path(log_path: &Path, index: usize) -> Result<PathBuf> {
    let file_name = log_path
        .file_name()
        .ok_or_else(|| anyhow!("daemon log path {} has no file name", log_path.display()))?;
    Ok(log_path.with_file_name(format!("{}.{index}", file_name.to_string_lossy())))
}

fn to_io_error(error: anyhow::Error) -> io::Error {
    io::Error::other(error)
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
            write_file(
                &archive_path(temp.path(), index),
                &format!("archive-{index}\n"),
            );
        }

        rotate_daemon_log_file(&current_log, 5).expect("rotate daemon log");

        assert_eq!(read_file(&archive_path(temp.path(), 1)), "current\n");
        assert_eq!(read_file(&archive_path(temp.path(), 2)), "archive-1\n");
        assert_eq!(read_file(&archive_path(temp.path(), 3)), "archive-2\n");
        assert_eq!(read_file(&archive_path(temp.path(), 4)), "archive-3\n");
        assert_eq!(read_file(&archive_path(temp.path(), 5)), "archive-4\n");
        assert!(!archive_path(temp.path(), 6).exists());
    }

    #[test]
    fn daemon_log_sink_keeps_oversized_append_in_active_file_until_next_write() {
        let temp = TempDir::new().expect("temp dir");
        let current_log = log_path(temp.path());
        let archive_1 = archive_path(temp.path(), 1);
        let oversized_entry = vec![b'x'; DAEMON_LOG_ROTATION_BYTES as usize + 1];

        let mut sink = open_daemon_log_sink(&current_log).expect("open daemon log sink");
        sink.write_all(&oversized_entry)
            .expect("write oversized daemon log entry");
        sink.flush().expect("flush oversized daemon log entry");

        assert_eq!(
            fs::metadata(&current_log)
                .expect("active daemon log metadata")
                .len(),
            oversized_entry.len() as u64
        );
        assert!(!archive_1.exists());

        sink.write_all(b"next-entry\n")
            .expect("write rollover boundary daemon log entry");
        sink.flush()
            .expect("flush rollover boundary daemon log entry");

        assert_eq!(
            fs::metadata(&archive_1)
                .expect("rotated daemon log archive metadata")
                .len(),
            oversized_entry.len() as u64
        );
        assert_eq!(read_file(&current_log), "next-entry\n");
    }
}
