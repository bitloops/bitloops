use super::*;

pub(super) fn launch_agent_plist_path(service_name: &str) -> Result<PathBuf> {
    Ok(user_home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{service_name}.plist")))
}

pub(super) fn systemd_user_unit_path(service_name: &str) -> Result<PathBuf> {
    Ok(user_home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{service_name}.service")))
}

pub(super) fn render_launchd_plist(
    service_name: &str,
    repo_root: &Path,
    executable: &Path,
    argv: &[OsString],
) -> String {
    let mut rendered = String::new();
    rendered.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    rendered.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    rendered.push_str("<plist version=\"1.0\">\n<dict>\n");
    rendered.push_str("  <key>Label</key>\n");
    rendered.push_str(&format!(
        "  <string>{}</string>\n",
        xml_escape(service_name)
    ));
    rendered.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    rendered.push_str(&format!(
        "    <string>{}</string>\n",
        xml_escape(&executable.to_string_lossy())
    ));
    for arg in argv {
        rendered.push_str(&format!(
            "    <string>{}</string>\n",
            xml_escape(&arg.to_string_lossy())
        ));
    }
    rendered.push_str("  </array>\n");
    rendered.push_str("  <key>WorkingDirectory</key>\n");
    rendered.push_str(&format!(
        "  <string>{}</string>\n",
        xml_escape(&repo_root.to_string_lossy())
    ));
    rendered.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    rendered.push_str("  <key>KeepAlive</key>\n  <true/>\n");
    rendered.push_str("</dict>\n</plist>\n");
    rendered
}

pub(super) fn render_systemd_unit(
    service_name: &str,
    repo_root: &Path,
    executable: &Path,
    argv: &[OsString],
) -> String {
    let exec_start = std::iter::once(executable.as_os_str().to_os_string())
        .chain(argv.iter().cloned())
        .map(|arg| systemd_escape_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[Unit]\nDescription=Bitloops daemon ({service_name})\n\n[Service]\nType=simple\nWorkingDirectory={}\nExecStart={exec_start}\nRestart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        repo_root.display()
    )
}

pub(super) fn render_windows_task_command(executable: &Path, argv: &[OsString]) -> String {
    std::iter::once(executable.as_os_str().to_os_string())
        .chain(argv.iter().cloned())
        .map(|arg| windows_escape_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_escape_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn windows_escape_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

pub(super) fn write_text_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving service file parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating service file directory {}", parent.display()))?;
    fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

pub(super) fn run_status_command(mut command: Command, action: &str) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| action.to_string())?;
    if !status.success() {
        bail!("{action} failed");
    }
    Ok(())
}
