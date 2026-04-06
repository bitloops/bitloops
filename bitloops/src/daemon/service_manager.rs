use super::*;

pub(super) fn install_or_update_supervisor_service() -> Result<SupervisorServiceMetadata> {
    let manager = current_service_manager();
    let service_name = GLOBAL_SUPERVISOR_SERVICE_NAME.to_string();
    let executable =
        env::current_exe().context("resolving Bitloops executable for supervisor service")?;
    let service_metadata_path = supervisor_service_metadata_path()?;
    let argv = vec![OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)];
    let working_directory = user_home_dir()?;

    let metadata = match manager {
        ServiceManagerKind::Launchd => {
            let path = launch_agent_plist_path(&service_name)?;
            let plist = render_launchd_plist(&service_name, &working_directory, &executable, &argv);
            write_text_file(&path, &plist)?;
            SupervisorServiceMetadata {
                version: 1,
                manager,
                service_name,
                service_file: Some(path),
            }
        }
        ServiceManagerKind::SystemdUser => {
            let path = systemd_user_unit_path(&service_name)?;
            let unit = render_systemd_unit(&service_name, &working_directory, &executable, &argv);
            write_text_file(&path, &unit)?;
            let mut command = Command::new("systemctl");
            command.arg("--user").arg("daemon-reload");
            run_status_command(
                command,
                "reloading systemd user units for Bitloops daemon supervisor",
            )?;
            SupervisorServiceMetadata {
                version: 1,
                manager,
                service_name,
                service_file: Some(path),
            }
        }
        ServiceManagerKind::WindowsTask => SupervisorServiceMetadata {
            version: 1,
            manager,
            service_name,
            service_file: None,
        },
    };

    let _ = service_metadata_path;
    write_supervisor_service_metadata(&metadata)?;
    Ok(metadata)
}

pub(super) fn start_configured_supervisor_service(
    metadata: &SupervisorServiceMetadata,
) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let path = metadata
                .service_file
                .as_ref()
                .context("missing launchd plist path for Bitloops daemon supervisor")?;
            let domain_target = launchd_domain_target()?;
            let _ = Command::new("launchctl")
                .arg("bootout")
                .arg(&domain_target)
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let mut bootstrap = Command::new("launchctl");
            bootstrap.arg("bootstrap").arg(&domain_target).arg(path);
            run_status_command(
                bootstrap,
                "bootstrapping Bitloops daemon supervisor launch agent",
            )?;
            let mut kickstart = Command::new("launchctl");
            kickstart
                .arg("kickstart")
                .arg("-k")
                .arg(format!("{domain_target}/{}", metadata.service_name));
            run_status_command(
                kickstart,
                "starting Bitloops daemon supervisor launch agent",
            )?;
        }
        ServiceManagerKind::SystemdUser => {
            let mut enable = Command::new("systemctl");
            enable
                .arg("--user")
                .arg("enable")
                .arg(&metadata.service_name);
            run_status_command(enable, "enabling Bitloops daemon supervisor user service")?;
            let mut restart = Command::new("systemctl");
            restart
                .arg("--user")
                .arg("restart")
                .arg(&metadata.service_name);
            run_status_command(restart, "starting Bitloops daemon supervisor user service")?;
        }
        ServiceManagerKind::WindowsTask => {
            let executable = env::current_exe()
                .context("resolving Bitloops executable for Windows supervisor task")?;
            let task_command = render_windows_task_command(
                &executable,
                &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
            );
            let mut create = Command::new("schtasks");
            create
                .arg("/Create")
                .arg("/TN")
                .arg(&metadata.service_name)
                .arg("/TR")
                .arg(task_command)
                .arg("/SC")
                .arg("ONLOGON")
                .arg("/F");
            run_status_command(
                create,
                "creating Windows scheduled task for Bitloops daemon supervisor",
            )?;
            let mut run = Command::new("schtasks");
            run.arg("/Run").arg("/TN").arg(&metadata.service_name);
            run_status_command(
                run,
                "starting Windows scheduled task for Bitloops daemon supervisor",
            )?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub(super) fn stop_configured_supervisor_service(
    metadata: &SupervisorServiceMetadata,
) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            let mut command = Command::new("launchctl");
            command.arg("bootout").arg(&domain_target);
            if let Some(path) = metadata.service_file.as_ref() {
                command.arg(path);
            } else {
                command.arg(format!("{domain_target}/{}", metadata.service_name));
            }
            run_status_command(command, "stopping Bitloops daemon supervisor launch agent")?;
        }
        ServiceManagerKind::SystemdUser => {
            let mut command = Command::new("systemctl");
            command
                .arg("--user")
                .arg("stop")
                .arg(&metadata.service_name);
            run_status_command(command, "stopping Bitloops daemon supervisor user service")?;
        }
        ServiceManagerKind::WindowsTask => {
            let mut command = Command::new("schtasks");
            command.arg("/End").arg("/TN").arg(&metadata.service_name);
            run_status_command(
                command,
                "stopping Windows scheduled task for Bitloops daemon supervisor",
            )?;
        }
    }
    Ok(())
}

pub(super) fn uninstall_configured_supervisor_service(
    metadata: &SupervisorServiceMetadata,
) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            if let Some(path) = metadata.service_file.as_ref() {
                let _ = Command::new("launchctl")
                    .arg("bootout")
                    .arg(&domain_target)
                    .arg(path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();

                if path.exists() {
                    fs::remove_file(path)
                        .with_context(|| format!("removing {}", path.display()))?;
                }
            } else {
                let _ = Command::new("launchctl")
                    .arg("bootout")
                    .arg(format!("{domain_target}/{}", metadata.service_name))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        ServiceManagerKind::SystemdUser => {
            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("stop")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("disable")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            if let Some(path) = metadata.service_file.as_ref()
                && path.exists()
            {
                fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
            }

            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("daemon-reload")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        ServiceManagerKind::WindowsTask => {
            let _ = Command::new("schtasks")
                .arg("/End")
                .arg("/TN")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("schtasks")
                .arg("/Delete")
                .arg("/TN")
                .arg(&metadata.service_name)
                .arg("/F")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    Ok(())
}

pub(super) fn is_supervisor_service_running(metadata: &SupervisorServiceMetadata) -> Result<bool> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            let status = Command::new("launchctl")
                .arg("print")
                .arg(format!("{domain_target}/{}", metadata.service_name))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("querying launchd Bitloops daemon supervisor status")?;
            Ok(status.success())
        }
        ServiceManagerKind::SystemdUser => {
            let status = Command::new("systemctl")
                .arg("--user")
                .arg("is-active")
                .arg("--quiet")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("querying systemd user Bitloops daemon supervisor status")?;
            Ok(status.success())
        }
        ServiceManagerKind::WindowsTask => {
            let output = Command::new("schtasks")
                .arg("/Query")
                .arg("/TN")
                .arg(&metadata.service_name)
                .output()
                .context("querying Windows scheduled task Bitloops daemon supervisor status")?;
            Ok(output.status.success())
        }
    }
}

pub(super) fn current_service_manager() -> ServiceManagerKind {
    #[cfg(target_os = "macos")]
    {
        return ServiceManagerKind::Launchd;
    }
    #[cfg(target_os = "linux")]
    {
        return ServiceManagerKind::SystemdUser;
    }
    #[cfg(target_os = "windows")]
    {
        return ServiceManagerKind::WindowsTask;
    }
    #[allow(unreachable_code)]
    ServiceManagerKind::Launchd
}

pub(super) fn launchd_domain_target() -> Result<String> {
    let uid = current_uid().context("resolving current uid for launchd user domain")?;
    Ok(format!("gui/{uid}"))
}

fn current_uid() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("running `id -u` for Bitloops daemon")?;
    if !output.status.success() {
        bail!("failed to resolve current uid for Bitloops daemon");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) fn user_home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("resolving user home directory for Bitloops daemon service files")
}
