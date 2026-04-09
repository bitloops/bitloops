use std::path::Path;

pub(crate) const PLAIN_TEXT_LANGUAGE_ID: &str = "plain_text";
const PLAIN_TEXT_MAX_BYTES: usize = 1024 * 1024;

pub(crate) fn indexing_language_for_path(path: &str) -> String {
    super::resolve_language_id_for_file_path(path)
        .unwrap_or(PLAIN_TEXT_LANGUAGE_ID)
        .to_string()
}

pub(crate) fn plain_text_content_is_allowed(content: &str) -> bool {
    if content.len() > PLAIN_TEXT_MAX_BYTES {
        return false;
    }
    if content.contains('\0') {
        return false;
    }
    // `git cat-file` currently uses lossy UTF-8 decoding; reject replacement-char output so
    // plain-text fallback stays UTF-8 only.
    if content.contains('\u{FFFD}') {
        return false;
    }
    true
}

pub(crate) fn should_skip_plain_text_fallback_path(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if let Some(file_name) = file_name
        && (file_name.ends_with(".sqlite-wal")
            || file_name.ends_with(".sqlite-shm")
            || file_name.ends_with(".db-wal")
            || file_name.ends_with(".db-shm"))
    {
        return true;
    }

    let Some(extension) = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return false;
    };

    matches!(
        extension.as_str(),
        "sqlite"
            | "sqlite3"
            | "db"
            | "wal"
            | "shm"
            | "lock"
            | "sqlite-wal"
            | "sqlite-shm"
            | "db-wal"
            | "db-shm"
            | "duckdb"
            | "bin"
            | "exe"
            | "dll"
            | "dylib"
            | "so"
            | "a"
            | "o"
            | "class"
            | "jar"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "ico"
            | "pdf"
            | "zip"
            | "gz"
            | "tar"
            | "tgz"
            | "bz2"
            | "xz"
            | "7z"
    )
}
