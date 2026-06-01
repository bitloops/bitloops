pub(in crate::daemon) struct ChildTerminationRecord {
    pub(in crate::daemon) pid: u32,
    pub(in crate::daemon) outcome: ChildTerminationOutcome,
}

impl ChildTerminationRecord {
    pub(in crate::daemon) fn summary(&self) -> String {
        self.outcome.summary()
    }

    pub(in crate::daemon) fn is_expected_shutdown(&self) -> bool {
        match self.outcome {
            ChildTerminationOutcome::Exited { code } => code == 0,
            ChildTerminationOutcome::Signaled { signal, .. } => {
                signal == expected_shutdown_signal_term()
                    || signal == expected_shutdown_signal_int()
            }
            ChildTerminationOutcome::Stopped { .. }
            | ChildTerminationOutcome::Continued
            | ChildTerminationOutcome::Unknown { .. } => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum ChildTerminationOutcome {
    Exited { code: i32 },
    Signaled { signal: i32, core_dumped: bool },
    Stopped { signal: i32 },
    Continued,
    Unknown { raw_status: i32 },
}

impl ChildTerminationOutcome {
    fn summary(&self) -> String {
        match self {
            ChildTerminationOutcome::Exited { code } => format!("exited with code {code}"),
            ChildTerminationOutcome::Signaled {
                signal,
                core_dumped,
            } => {
                let signal_name = signal_name(*signal)
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default();
                if *core_dumped {
                    format!("signal {signal}{signal_name}, core dumped")
                } else {
                    format!("signal {signal}{signal_name}")
                }
            }
            ChildTerminationOutcome::Stopped { signal } => {
                let signal_name = signal_name(*signal)
                    .map(|name| format!(" ({name})"))
                    .unwrap_or_default();
                format!("stopped by signal {signal}{signal_name}")
            }
            ChildTerminationOutcome::Continued => "continued".to_string(),
            ChildTerminationOutcome::Unknown { raw_status } => {
                format!("unknown wait status {raw_status}")
            }
        }
    }
}

#[cfg(unix)]
fn signal_name(signal: i32) -> Option<&'static str> {
    match signal {
        libc::SIGTERM => Some("SIGTERM"),
        libc::SIGINT => Some("SIGINT"),
        libc::SIGKILL => Some("SIGKILL"),
        libc::SIGABRT => Some("SIGABRT"),
        libc::SIGSEGV => Some("SIGSEGV"),
        libc::SIGBUS => Some("SIGBUS"),
        libc::SIGILL => Some("SIGILL"),
        libc::SIGQUIT => Some("SIGQUIT"),
        libc::SIGTRAP => Some("SIGTRAP"),
        _ => None,
    }
}

#[cfg(not(unix))]
fn signal_name(_signal: i32) -> Option<&'static str> {
    None
}

fn expected_shutdown_signal_term() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGTERM
    }
    #[cfg(not(unix))]
    {
        15
    }
}

fn expected_shutdown_signal_int() -> i32 {
    #[cfg(unix)]
    {
        libc::SIGINT
    }
    #[cfg(not(unix))]
    {
        2
    }
}
