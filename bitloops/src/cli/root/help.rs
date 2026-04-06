use anyhow::Result;
use clap::{Command, CommandFactory};
use std::io::Write;

pub(crate) fn write_help(
    w: &mut dyn Write,
    command_path: &[String],
    show_tree: bool,
) -> Result<()> {
    let root = crate::cli::Cli::command();
    if show_tree {
        return write_command_tree(w, &root);
    }

    let mut target = find_target_command(&root, command_path).clone();
    let mut rendered = Vec::new();
    target.write_long_help(&mut rendered)?;
    w.write_all(&rendered)?;
    writeln!(w)?;
    Ok(())
}

fn find_target_command<'a>(root: &'a Command, command_path: &[String]) -> &'a Command {
    if command_path.is_empty() {
        return root;
    }

    let mut current = root;
    for name in command_path.iter().filter(|value| !value.is_empty()) {
        let Some(next) = current.get_subcommands().find(|sub| sub.get_name() == name) else {
            return root;
        };
        current = next;
    }

    current
}

fn write_command_tree(w: &mut dyn Write, root: &Command) -> Result<()> {
    writeln!(w, "{}", root.get_name())?;
    write_children(w, root, "")
}

fn write_children(w: &mut dyn Write, cmd: &Command, indent: &str) -> Result<()> {
    let visible: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .collect();

    for (idx, sub) in visible.iter().enumerate() {
        let is_last = idx == visible.len().saturating_sub(1);
        write_node(w, sub, indent, is_last)?;
    }

    Ok(())
}

fn write_node(w: &mut dyn Write, cmd: &Command, indent: &str, is_last: bool) -> Result<()> {
    let (branch, child_indent) = if is_last {
        ("└── ", format!("{indent}    "))
    } else {
        ("├── ", format!("{indent}│   "))
    };

    write!(w, "{indent}{branch}{}", cmd.get_name())?;
    if let Some(short) = cmd.get_about().map(|about| about.to_string()) {
        let short = short.trim();
        if !short.is_empty() {
            write!(w, " - {short}")?;
        }
    }
    writeln!(w)?;

    write_children(w, cmd, &child_indent)
}
