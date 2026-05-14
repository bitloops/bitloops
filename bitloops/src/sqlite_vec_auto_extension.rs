use anyhow::{Result, anyhow};
use std::ffi::{c_char, c_int};
use std::sync::OnceLock;

unsafe extern "C" {
    #[link_name = "sqlite3_vec_init"]
    fn sqlite3_vec_init_raw(
        db: *mut rusqlite::ffi::sqlite3,
        pz_err_msg: *mut *mut c_char,
        api: *const rusqlite::ffi::sqlite3_api_routines,
    ) -> c_int;
}
pub(crate) fn register_sqlite_vec_auto_extension() -> Result<()> {
    static REGISTRATION: OnceLock<std::result::Result<(), String>> = OnceLock::new();

    match REGISTRATION.get_or_init(register_sqlite_vec_auto_extension_once) {
        Ok(()) => Ok(()),
        Err(err) => Err(anyhow!(err.clone())),
    }
}

fn register_sqlite_vec_auto_extension_once() -> std::result::Result<(), String> {
    let extension = raw_sqlite_vec_auto_extension();
    // Safety: sqlite-vec exposes the canonical SQLite extension entrypoint, and we
    // register it once process-wide so each later SQLite connection auto-loads it.
    unsafe { rusqlite::auto_extension::register_auto_extension(extension) }
        .map_err(|err| format!("registering sqlite-vec auto-extension: {err}"))
}

fn raw_sqlite_vec_auto_extension() -> rusqlite::auto_extension::RawAutoExtension {
    let _ = sqlite_vec::sqlite3_vec_init as *const ();
    sqlite3_vec_init_raw
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    #[test]
    fn register_sqlite_vec_auto_extension_makes_vec_version_available_on_new_connections()
    -> Result<()> {
        super::register_sqlite_vec_auto_extension()?;
        super::register_sqlite_vec_auto_extension()?;

        let conn = rusqlite::Connection::open_in_memory()
            .context("opening in-memory SQLite connection")?;
        let version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .context("querying sqlite-vec version from a fresh SQLite connection")?;

        assert!(version.starts_with('v'));
        Ok(())
    }
}
