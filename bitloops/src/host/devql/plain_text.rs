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
