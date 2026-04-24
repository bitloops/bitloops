use std::env;
#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

pub(super) fn can_prompt_interactively() -> bool {
    if let Ok(value) = env::var("BITLOOPS_TEST_TTY") {
        return value == "1" && command_exists("stty");
    }

    #[cfg(test)]
    {
        false
    }

    #[cfg(not(test))]
    {
        if io::stdin().is_terminal() && io::stdout().is_terminal() && command_exists("stty") {
            return true;
        }

        fs::OpenOptions::new().read(true).open("/dev/tty").is_ok() && command_exists("stty")
    }
}

fn command_exists(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
            || executable_with_extensions(&dir, program)
                .iter()
                .any(|candidate| candidate.is_file())
    })
}

fn executable_with_extensions(dir: &Path, program: &str) -> [PathBuf; 3] {
    [
        dir.join(format!("{program}.exe")),
        dir.join(format!("{program}.cmd")),
        dir.join(format!("{program}.bat")),
    ]
}
