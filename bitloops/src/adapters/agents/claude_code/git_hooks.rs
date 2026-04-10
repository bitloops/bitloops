//! Git hook installation / uninstallation for the manual-commit strategy.
//!
//! Installs 6-7 shell scripts into `.git/hooks/` (or `core.hooksPath`):
//!   prepare-commit-msg, commit-msg, post-commit, post-merge, post-checkout, pre-push,
//!   reference-transaction (Git >= 2.28)
//!
//! Each script calls `bitloops hooks git <verb>` and can chain to a
//! pre-existing hook backed up with the `.pre-bitloops` suffix.

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[cfg(test)]
use crate::test_support::process_state::git_command;

/// Comment embedded in every managed hook script — used as the installation marker.
const HOOK_MARKER: &str = "# Bitloops git hooks";

/// Suffix appended to pre-existing hooks when backing them up.
const BACKUP_SUFFIX: &str = ".pre-bitloops";

const REFERENCE_TRANSACTION_HOOK: &str = "reference-transaction";
const MIN_REFERENCE_TRANSACTION_GIT_VERSION: (u32, u32) = (2, 28);
const HOOK_GIT_ENV_KEYS: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
];

/// Hook set that is always installed regardless of git version.
static BASE_HOOK_NAMES: &[&str] = &[
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "post-merge",
    "post-checkout",
    "pre-push",
];

/// All managed git hooks (reference-transaction is version-gated at install time).
static HOOK_NAMES: &[&str] = &[
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "post-merge",
    "post-checkout",
    "pre-push",
    REFERENCE_TRANSACTION_HOOK,
];

/// External hook-manager metadata detected in the repo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HookManager {
    name: String,
    config_path: String,
    overwrites_hooks: bool,
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn new_git_command() -> Command {
    #[cfg(test)]
    {
        git_command()
    }

    #[cfg(not(test))]
    {
        Command::new("git")
    }
}

/// Returns the active git hooks directory path via `git rev-parse --git-path hooks`.
/// Respects `core.hooksPath` and linked worktrees.
fn get_git_dir(repo_root: &Path) -> Result<PathBuf> {
    let output = new_git_command()
        .args(["rev-parse", "--git-dir"])
        .current_dir(repo_root)
        .output()
        .context("running `git rev-parse --git-dir`")?;

    if !output.status.success() {
        anyhow::bail!("not a git repository");
    }

    let raw = std::str::from_utf8(&output.stdout)
        .context("git output is not utf-8")?
        .trim();

    let git_dir = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        repo_root.join(raw)
    };

    Ok(git_dir)
}

fn get_hooks_dir(repo_root: &Path) -> Result<PathBuf> {
    // Fail in non-repo directories.
    let _ = get_git_dir(repo_root)?;

    let output = new_git_command()
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(repo_root)
        .output()
        .context("running `git rev-parse --git-path hooks`")?;

    if !output.status.success() {
        anyhow::bail!("not a git repository");
    }

    let raw = std::str::from_utf8(&output.stdout)
        .context("git output is not utf-8")?
        .trim()
        .to_string();

    let hooks_dir = if Path::new(&raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        repo_root.join(raw)
    };

    Ok(hooks_dir)
}

/// Returns the command prefix used in hook scripts.
fn hook_cmd_prefix(local_dev: bool) -> &'static str {
    // Both modes use the installed binary — local dev is handled separately.
    let _ = local_dev;
    "bitloops"
}

fn git_hook_env_sanitizer() -> String {
    let mut script =
        "# Clear inherited git-hook state before Bitloops runs nested git commands.\n".to_string();
    for key in HOOK_GIT_ENV_KEYS {
        script.push_str(&format!("unset {key}\n"));
    }
    script
}

struct HookSpec {
    name: &'static str,
    content: String,
}

/// Builds a small shell snippet that lets hook-invoked `bitloops` locate
/// dynamically linked DuckDB runtimes in development test builds.
fn duckdb_runtime_linker_bootstrap(cmd_prefix: &str) -> String {
    format!(
        "# Resolve DuckDB runtime library when using dynamic (non-bundled) builds.\n\
         _bitloops_add_lib_path() {{\n\
             _candidate=\"$1\"\n\
             if [ ! -d \"$_candidate\" ]; then\n\
                 return 1\n\
             fi\n\
             if [ -f \"$_candidate/libduckdb.dylib\" ]; then\n\
                 case \":${{DYLD_LIBRARY_PATH:-}}:\" in\n\
                     *\":$_candidate:\"*) ;;\n\
                     *) export DYLD_LIBRARY_PATH=\"$_candidate${{DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}}\" ;;\n\
                 esac\n\
                 return 0\n\
             fi\n\
             if [ -f \"$_candidate/libduckdb.so\" ]; then\n\
                 case \":${{LD_LIBRARY_PATH:-}}:\" in\n\
                     *\":$_candidate:\"*) ;;\n\
                     *) export LD_LIBRARY_PATH=\"$_candidate${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}\" ;;\n\
                 esac\n\
                 return 0\n\
             fi\n\
             return 1\n\
         }}\n\
         _bitloops_bin=\"$(command -v \"{cmd_prefix}\" 2>/dev/null || true)\"\n\
         if [ -n \"$_bitloops_bin\" ]; then\n\
             _bitloops_bin_dir=\"$(cd \"$(dirname \"$_bitloops_bin\")\" && pwd)\"\n\
             if ! _bitloops_add_lib_path \"$_bitloops_bin_dir/deps\"; then\n\
                 if ! _bitloops_add_lib_path \"$_bitloops_bin_dir/../deps\"; then\n\
                     for _candidate in \"$_bitloops_bin_dir\"/../duckdb-download/*/*; do\n\
                         _bitloops_add_lib_path \"$_candidate\" && break\n\
                     done\n\
                 fi\n\
             fi\n\
         fi\n"
    )
}

/// Builds the content of all managed hook scripts.
fn build_hook_specs(cmd_prefix: &str) -> Vec<HookSpec> {
    let runtime_bootstrap = duckdb_runtime_linker_bootstrap(cmd_prefix);
    let daemon_config_export = crate::adapters::agents::managed_hook_env_export_script();
    let git_env_sanitizer = git_hook_env_sanitizer();
    vec![
        HookSpec {
            name: "prepare-commit-msg",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 {cmd_prefix} hooks git prepare-commit-msg \"$1\" \"$2\" 2>/dev/null || true\n"
            ),
        },
        HookSpec {
            name: "commit-msg",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Commit-msg: `bitloops hooks git commit-msg` (default manual-commit: no-op)\n\
                 {cmd_prefix} hooks git commit-msg \"$1\" || exit 1\n"
            ),
        },
        HookSpec {
            name: "post-commit",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Post-commit: session/checkpoint bookkeeping; failures must not block git\n\
                 {cmd_prefix} hooks git post-commit 2>/dev/null || true\n"
            ),
        },
        HookSpec {
            name: "post-checkout",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Post-checkout: branch seeding and bookkeeping; failures must not block git\n\
                 {cmd_prefix} hooks git post-checkout \"$@\" 2>/dev/null || true\n"
            ),
        },
        HookSpec {
            name: "post-merge",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Post-merge: refresh DevQL after pull/merge; failures must not block git\n\
                 {cmd_prefix} hooks git post-merge \"$@\" 2>/dev/null || true\n"
            ),
        },
        HookSpec {
            name: "pre-push",
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Pre-push: `bitloops hooks git pre-push` (default manual-commit: no-op)\n\
                 # $1 is the remote name (e.g., \"origin\")\n\
                 {cmd_prefix} hooks git pre-push \"$1\" || true\n"
            ),
        },
        HookSpec {
            name: REFERENCE_TRANSACTION_HOOK,
            content: format!(
                "#!/bin/sh\n{HOOK_MARKER}\n\
                 {runtime_bootstrap}\
                 {daemon_config_export}\
                 {git_env_sanitizer}\
                 # Reference-transaction: branch deletion cleanup; failures must not block git\n\
                 {cmd_prefix} hooks git reference-transaction \"$@\" 2>/dev/null || true\n"
            ),
        },
    ]
}

fn parse_git_version(version_output: &str) -> Option<(u32, u32, u32)> {
    let token = version_output
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))?;
    let mut components = token.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components.next().and_then(|raw| {
        let digits: String = raw.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.is_empty() {
            Some(0)
        } else {
            digits.parse().ok()
        }
    })?;
    Some((major, minor, patch))
}

fn git_supports_reference_transaction(repo_root: &Path) -> bool {
    let output = new_git_command()
        .args(["version"])
        .current_dir(repo_root)
        .output();
    let Ok(output) = output else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let Some((major, minor, _patch)) = parse_git_version(raw.trim()) else {
        return true;
    };
    (major, minor) >= MIN_REFERENCE_TRANSACTION_GIT_VERSION
}

fn expected_hooks_for_repo(repo_root: &Path) -> Vec<&'static str> {
    if git_supports_reference_transaction(repo_root) {
        HOOK_NAMES.to_vec()
    } else {
        BASE_HOOK_NAMES.to_vec()
    }
}

/// Detects known third-party hook managers by config file/directory presence.
fn detect_hook_managers(repo_root: &Path) -> Vec<HookManager> {
    let mut checks = vec![
        HookManager {
            name: "Husky".to_string(),
            config_path: ".husky/".to_string(),
            overwrites_hooks: true,
        },
        HookManager {
            name: "pre-commit".to_string(),
            config_path: ".pre-commit-config.yaml".to_string(),
            overwrites_hooks: false,
        },
        HookManager {
            name: "Overcommit".to_string(),
            config_path: ".overcommit.yml".to_string(),
            overwrites_hooks: false,
        },
    ];

    // Lefthook supports {.,}lefthook{,-local}.{yml,yaml,json,toml}
    for prefix in ["", "."] {
        for variant in ["", "-local"] {
            for ext in ["yml", "yaml", "json", "toml"] {
                checks.push(HookManager {
                    name: "Lefthook".to_string(),
                    config_path: format!("{prefix}lefthook{variant}.{ext}"),
                    overwrites_hooks: false,
                });
            }
        }
    }

    let mut seen_names = std::collections::BTreeSet::new();
    let mut managers = Vec::new();
    for check in checks {
        if repo_root.join(&check.config_path).exists() && seen_names.insert(check.name.clone()) {
            managers.push(check);
        }
    }

    managers
}

/// Returns the first non-comment/non-shebang command line from hook content.
fn extract_command_line(hook_content: &str) -> String {
    for line in hook_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.contains(" hooks git ") {
            return trimmed.to_string();
        }
    }

    for line in hook_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return trimmed.to_string();
    }
    String::new()
}

/// Builds warning text for detected hook managers.
fn hook_manager_warning(managers: &[HookManager], cmd_prefix: &str) -> String {
    if managers.is_empty() {
        return String::new();
    }

    let specs = build_hook_specs(cmd_prefix);
    let mut out = String::new();

    for manager in managers {
        if manager.overwrites_hooks {
            out.push_str(&format!(
                "Warning: {} detected ({})\n\n",
                manager.name, manager.config_path
            ));
            out.push_str(&format!(
                "  {} may overwrite hooks installed by Bitloops on npm install.\n",
                manager.name
            ));
            out.push_str(&format!(
                "  To make Bitloops hooks permanent, add these lines to your {} hook files:\n\n",
                manager.name
            ));

            for spec in &specs {
                let cmd_line = extract_command_line(&spec.content);
                if cmd_line.is_empty() {
                    continue;
                }
                out.push_str(&format!("    {}{}:\n", manager.config_path, spec.name));
                out.push_str(&format!("      {}\n\n", cmd_line));
            }
        } else {
            out.push_str(&format!(
                "Note: {} detected ({})\n\n",
                manager.name, manager.config_path
            ));
            out.push_str(&format!(
                "  If {} reinstalls hooks, run 'bitloops enable' to restore Bitloops hooks.\n\n",
                manager.name
            ));
        }
    }

    out
}

/// Detects external hook managers from repo root and prints warning text when needed.
pub fn check_and_warn_hook_managers(w: &mut dyn Write, local_dev: bool) {
    let repo_root = match crate::utils::paths::repo_root() {
        Ok(root) => root,
        Err(_) => return,
    };

    let managers = detect_hook_managers(&repo_root);
    if managers.is_empty() {
        return;
    }

    let warning = hook_manager_warning(&managers, hook_cmd_prefix(local_dev));
    if !warning.is_empty() {
        let _ = writeln!(w);
        let _ = write!(w, "{warning}");
    }
}

/// Appends a chained call to the backed-up hook at the end of the script content.
fn generate_chained_content(base_content: &str, hook_name: &str) -> String {
    format!(
        "{base_content}\
         # Chain: run pre-existing hook\n\
         _bitloops_hook_dir=\"$(dirname \"$0\")\"\n\
         if [ -x \"$_bitloops_hook_dir/{hook_name}{BACKUP_SUFFIX}\" ]; then\n\
             \"$_bitloops_hook_dir/{hook_name}{BACKUP_SUFFIX}\" \"$@\"\n\
         fi\n"
    )
}

/// Writes `content` to `path` with executable permissions.
/// Returns `true` if the file was written (content changed), `false` if already up to date.
fn write_hook_file(path: &Path, content: &str) -> Result<bool> {
    // Skip if already up to date.
    if let Ok(existing) = fs::read_to_string(path)
        && existing == content
    {
        return Ok(false);
    }

    fs::write(path, content).with_context(|| format!("writing hook: {}", path.display()))?;

    // Make executable (Unix only — on Windows git hooks are sh scripts anyway).
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)
            .context("getting hook file metadata")?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).context("setting hook file executable")?;
    }

    Ok(true)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Installs the managed git hook scripts into `.git/hooks/`.
///
/// Pre-existing hooks that don't contain the marker are backed up to
/// `<hook-name>.pre-bitloops` and chained at the end of the new script.
///
/// Returns the number of hook files written (0 if all already up to date).
///
pub fn install_git_hooks(repo_root: &Path, local_dev: bool) -> Result<usize> {
    let hooks_dir = get_hooks_dir(repo_root)?;
    fs::create_dir_all(&hooks_dir).context("creating git hooks directory")?;

    let cmd_prefix = hook_cmd_prefix(local_dev);
    let supports_reference_transaction = git_supports_reference_transaction(repo_root);
    if !supports_reference_transaction {
        eprintln!(
            "[bitloops] Warning: git reference-transaction hook requires Git 2.28+; skipping installation."
        );
    }
    let specs = build_hook_specs(cmd_prefix)
        .into_iter()
        .filter(|spec| supports_reference_transaction || spec.name != REFERENCE_TRANSACTION_HOOK)
        .collect::<Vec<_>>();
    let mut installed_count = 0;

    for spec in &specs {
        let hook_path = hooks_dir.join(spec.name);
        let backup_path = hooks_dir.join(format!("{}{BACKUP_SUFFIX}", spec.name));
        let mut backup_exists = backup_path.exists();

        // Back up a pre-existing hook that doesn't belong to us.
        if let Ok(existing) = fs::read_to_string(&hook_path)
            && !existing.contains(HOOK_MARKER)
        {
            if !backup_exists {
                fs::rename(&hook_path, &backup_path)
                    .with_context(|| format!("backing up {}", spec.name))?;
                eprintln!(
                    "[bitloops] Backed up existing {} to {}{}",
                    spec.name, spec.name, BACKUP_SUFFIX
                );
            } else {
                eprintln!(
                    "[bitloops] Warning: replacing {} (backup {}{} already exists)",
                    spec.name, spec.name, BACKUP_SUFFIX
                );
            }
            backup_exists = true;
        }

        // Chain to backup if one exists.
        let content = if backup_exists {
            generate_chained_content(&spec.content, spec.name)
        } else {
            spec.content.clone()
        };

        if write_hook_file(&hook_path, &content)? {
            installed_count += 1;
        }
    }

    Ok(installed_count)
}

/// Removes all Bitloops-managed git hook scripts.
/// Restores `.pre-bitloops` backups if they exist.
///
/// Returns the number of hooks removed.
///
pub fn uninstall_git_hooks(repo_root: &Path) -> Result<usize> {
    let hooks_dir = get_hooks_dir(repo_root)?;

    let mut removed = 0;

    for &hook_name in HOOK_NAMES {
        let hook_path = hooks_dir.join(hook_name);
        let backup_path = hooks_dir.join(format!("{hook_name}{BACKUP_SUFFIX}"));

        let content = fs::read_to_string(&hook_path).unwrap_or_default();
        let hook_is_ours = content.contains(HOOK_MARKER);
        let hook_exists = hook_path.exists();

        if hook_is_ours {
            fs::remove_file(&hook_path).with_context(|| format!("removing hook {hook_name}"))?;
            removed += 1;
        }

        // Restore backup if it exists.
        if backup_path.exists() {
            if hook_exists && !hook_is_ours {
                // A foreign hook is present — leave the backup in place.
                eprintln!(
                    "[bitloops] Warning: {hook_name} was modified since install; \
                     backup {hook_name}{BACKUP_SUFFIX} left in place"
                );
            } else {
                fs::rename(&backup_path, &hook_path)
                    .with_context(|| format!("restoring {hook_name}{BACKUP_SUFFIX}"))?;
            }
        }
    }

    Ok(removed)
}

/// Returns `true` if all Bitloops git hook scripts are installed.
pub fn is_git_hook_installed(repo_root: &Path) -> bool {
    let hooks_dir = match get_hooks_dir(repo_root) {
        Ok(d) => d,
        Err(_) => return false,
    };
    expected_hooks_for_repo(repo_root).iter().all(|name| {
        fs::read_to_string(hooks_dir.join(name))
            .map(|c| c.contains(HOOK_MARKER))
            .unwrap_or(false)
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "git_hooks_tests.rs"]
mod tests;
