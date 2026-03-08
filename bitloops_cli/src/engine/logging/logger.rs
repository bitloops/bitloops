use super::context::{LogContext, attrs_from_context};
use anyhow::Result;
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOG_LEVEL_ENV_VAR: &str = "ENTIRE_LOG_LEVEL";
pub const LOGS_DIR: &str = ".bitloops/logs";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub key: String,
    pub value: Value,
}

impl Attr {
    pub fn as_string(&self) -> String {
        match &self.value {
            Value::String(s) => s.clone(),
            _ => self.value.to_string(),
        }
    }
}

pub fn string_attr(key: &str, value: &str) -> Attr {
    Attr {
        key: key.to_string(),
        value: Value::String(value.to_string()),
    }
}

pub fn int_attr(key: &str, value: i64) -> Attr {
    Attr {
        key: key.to_string(),
        value: Value::from(value),
    }
}

pub fn bool_attr(key: &str, value: bool) -> Attr {
    Attr {
        key: key.to_string(),
        value: Value::from(value),
    }
}

#[derive(Default)]
struct LoggerState {
    initialized: bool,
    level: LogLevel,
    current_session_id: String,
    output: OutputSink,
    last_log_entry: Option<Value>,
    stderr_capture: Option<String>,
}

#[derive(Default)]
enum OutputSink {
    #[default]
    None,
    File(BufWriter<File>),
    Stderr,
}

fn logger_state() -> &'static Mutex<LoggerState> {
    static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(LoggerState::default()))
}

pub fn init(session_id: &str) -> Result<()> {
    if !session_id.is_empty() {
        validate_session_id(session_id)?;
    }

    let level_raw = std::env::var(LOG_LEVEL_ENV_VAR).unwrap_or_default();
    let level = parse_log_level(&level_raw);

    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    close_locked(&mut state);

    state.initialized = true;
    state.level = level;
    state.last_log_entry = None;

    if !level_raw.is_empty() && !is_valid_log_level(&level_raw) {
        write_stderr_line_locked(
            &mut state,
            &format!(
                "[bitloops] Warning: invalid log level {:?}, defaulting to INFO",
                level_raw
            ),
        );
    }

    let repo_root = repo_root();
    let logs_dir = std::path::Path::new(&repo_root).join(LOGS_DIR);
    if std::fs::create_dir_all(&logs_dir).is_err() {
        state.output = OutputSink::Stderr;
        state.current_session_id.clear();
        return Ok(());
    }

    let log_path = logs_dir.join("bitloops.log");
    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(file) => {
            state.output = OutputSink::File(BufWriter::with_capacity(8192, file));
            state.current_session_id = session_id.to_string();
        }
        Err(_) => {
            state.output = OutputSink::Stderr;
            state.current_session_id.clear();
        }
    }

    Ok(())
}

pub fn close() {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    close_locked(&mut state);
}

pub fn reset_logger_for_tests() {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    close_locked(&mut state);
    state.stderr_capture = None;
}

pub fn set_test_logger_without_global_session_for_tests() {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    close_locked(&mut state);
    state.initialized = true;
    state.level = LogLevel::Info;
    state.output = OutputSink::Stderr;
    state.current_session_id.clear();
    state.last_log_entry = None;
}

pub fn take_last_log_entry_for_tests() -> Option<Value> {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.last_log_entry.take()
}

pub fn start_stderr_capture_for_tests() {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.stderr_capture = Some(String::new());
}

pub fn take_stderr_capture_for_tests() -> String {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.stderr_capture.take().unwrap_or_default()
}

pub fn parse_log_level(value: &str) -> LogLevel {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEBUG" => LogLevel::Debug,
        "INFO" => LogLevel::Info,
        "WARN" | "WARNING" => LogLevel::Warn,
        "ERROR" => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

pub fn debug(ctx: &LogContext, msg: &str, attrs: &[Attr]) {
    log(ctx, LogLevel::Debug, msg, attrs);
}

pub fn info(ctx: &LogContext, msg: &str, attrs: &[Attr]) {
    log(ctx, LogLevel::Info, msg, attrs);
}

pub fn warn(ctx: &LogContext, msg: &str, attrs: &[Attr]) {
    log(ctx, LogLevel::Warn, msg, attrs);
}

pub fn error(ctx: &LogContext, msg: &str, attrs: &[Attr]) {
    log(ctx, LogLevel::Error, msg, attrs);
}

pub fn log_duration(
    ctx: &LogContext,
    level: LogLevel,
    msg: &str,
    _start: SystemTime,
    attrs: &[Attr],
) {
    let duration_ms = SystemTime::now()
        .duration_since(_start)
        .unwrap_or_default()
        .as_millis() as i64;

    let mut combined = Vec::with_capacity(attrs.len() + 1);
    combined.push(int_attr("duration_ms", duration_ms));
    combined.extend_from_slice(attrs);

    log(ctx, level, msg, &combined);
}

fn log(ctx: &LogContext, level: LogLevel, msg: &str, attrs: &[Attr]) {
    let mut state = logger_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let minimum_level = if state.initialized {
        state.level
    } else {
        LogLevel::Info
    };
    if !should_log(level, minimum_level) {
        return;
    }

    let mut obj = serde_json::Map::<String, Value>::new();
    obj.insert("time".to_string(), Value::from(current_time_millis()));
    obj.insert(
        "level".to_string(),
        Value::String(level_as_str(level).to_string()),
    );
    obj.insert("msg".to_string(), Value::String(msg.to_string()));

    if !state.current_session_id.is_empty() {
        obj.insert(
            "session_id".to_string(),
            Value::String(state.current_session_id.clone()),
        );
    }

    let context_attrs = attrs_from_context(ctx, &state.current_session_id);
    for attr in context_attrs {
        obj.insert(attr.key, attr.value);
    }

    for attr in attrs {
        obj.insert(attr.key.clone(), attr.value.clone());
    }

    let entry = Value::Object(obj);
    state.last_log_entry = Some(entry.clone());
    let serialized = match serde_json::to_string(&entry) {
        Ok(v) => v,
        Err(_) => return,
    };

    if !state.initialized {
        write_stderr_line_locked(&mut state, &serialized);
        return;
    }

    let mut fallback_to_stderr = false;
    match &mut state.output {
        OutputSink::File(writer) => {
            if writer.write_all(serialized.as_bytes()).is_err()
                || writer.write_all(b"\n").is_err()
                || writer.flush().is_err()
            {
                fallback_to_stderr = true;
            }
        }
        OutputSink::Stderr | OutputSink::None => {
            write_stderr_line_locked(&mut state, &serialized);
        }
    }

    if fallback_to_stderr {
        state.output = OutputSink::Stderr;
        write_stderr_line_locked(&mut state, &serialized);
    }
}

fn close_locked(state: &mut LoggerState) {
    if let OutputSink::File(writer) = &mut state.output {
        let _ = writer.flush();
    }
    state.output = OutputSink::None;
    state.initialized = false;
    state.current_session_id.clear();
    state.last_log_entry = None;
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.contains('/') || session_id.contains('\\') {
        anyhow::bail!("invalid session ID: must not contain path separators");
    }
    Ok(())
}

fn repo_root() -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(output) = output
        && output.status.success()
    {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !root.is_empty() {
            return root;
        }
    }
    ".".to_string()
}

fn is_valid_log_level(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_uppercase().as_str(),
        "" | "DEBUG" | "INFO" | "WARN" | "WARNING" | "ERROR"
    )
}

fn should_log(level: LogLevel, minimum_level: LogLevel) -> bool {
    level_rank(level) >= level_rank(minimum_level)
}

fn level_rank(level: LogLevel) -> i32 {
    match level {
        LogLevel::Debug => 10,
        LogLevel::Info => 20,
        LogLevel::Warn => 30,
        LogLevel::Error => 40,
    }
}

fn level_as_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn write_stderr_line_locked(state: &mut LoggerState, line: &str) {
    if let Some(buf) = state.stderr_capture.as_mut() {
        buf.push_str(line);
        buf.push('\n');
    }
    eprintln!("{line}");
}
