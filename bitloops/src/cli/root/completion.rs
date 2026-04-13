use anyhow::Result;
use clap::CommandFactory;
use clap_complete::generate;
use std::io::{self, Write};

use super::args::{CompletionArgs, CompletionShell};
use super::metadata::ROOT_NAME;

pub(crate) fn write_completion(w: &mut dyn Write, shell: CompletionShell) -> Result<()> {
    let mut cmd = crate::cli::Cli::command();
    // clap_complete splits subcommand paths using "__". Our hidden
    // "__send_analytics", "__devql-watcher", and daemon internal commands
    // conflict with that separator and can panic during completion generation,
    // so we rename them only in this generated tree. Runtime parsing remains
    // unchanged.
    cmd = cmd.mut_subcommand("__devql-watcher", |sub| {
        sub.name("devql-watcher-internal")
            .bin_name(format!("{ROOT_NAME} devql-watcher-internal"))
    });
    cmd = cmd.mut_subcommand("__daemon-process", |sub| {
        sub.name("daemon-process-internal")
            .bin_name(format!("{ROOT_NAME} daemon-process-internal"))
    });
    cmd = cmd.mut_subcommand("__daemon-supervisor", |sub| {
        sub.name("daemon-supervisor-internal")
            .bin_name(format!("{ROOT_NAME} daemon-supervisor-internal"))
    });
    cmd = cmd.mut_subcommand("__send_analytics", |sub| {
        sub.name("send-analytics-internal")
            .bin_name(format!("{ROOT_NAME} send-analytics-internal"))
    });
    cmd.build();
    match shell {
        CompletionShell::Bash => generate(clap_complete::Shell::Bash, &mut cmd, ROOT_NAME, w),
        CompletionShell::Zsh => generate(clap_complete::Shell::Zsh, &mut cmd, ROOT_NAME, w),
        CompletionShell::Fish => generate(clap_complete::Shell::Fish, &mut cmd, ROOT_NAME, w),
    }
    Ok(())
}

pub fn run_completion_command(args: &CompletionArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write_completion(&mut out, args.shell)
}
