use super::root::{
    CompletionShell, ROOT_NAME, ROOT_SHORT_ABOUT, build_commit, build_date, build_target,
    build_version, has_hidden_in_chain, run_curl_bash_post_install_command_with_io,
    should_attempt_watcher_autostart, telemetry_action_for_command, write_completion, write_help,
    write_version,
};
use super::{Cli, Commands, resolve_watcher_autostart_config_root};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, ENV_DAEMON_CONFIG_PATH_OVERRIDE, REPO_POLICY_LOCAL_FILE_NAME,
};
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::{with_env_var, with_env_vars};
use crate::utils::branding::bitloops_wordmark;
use clap::{Command, CommandFactory, Parser};
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn find_subcommand<'a>(cmd: &'a Command, name: &str) -> &'a Command {
    cmd.get_subcommands()
        .find(|sub| sub.get_name() == name)
        .unwrap_or_else(|| panic!("could not find subcommand {name} under {}", cmd.get_name()))
}

fn render_long_help(mut cmd: Command) -> String {
    let mut out = Vec::new();
    cmd.write_long_help(&mut out)
        .expect("long help should render");
    String::from_utf8(out).expect("help should be valid utf-8")
}

fn render_custom_help(path: &[&str], show_tree: bool) -> String {
    let mut out = Vec::new();
    let command_path = path.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    write_help(&mut out, &command_path, show_tree).expect("custom help should render");
    String::from_utf8(out).expect("help should be valid utf-8")
}

fn write_test_daemon_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    std::fs::write(
        &config_path,
        r#"[runtime]
local_dev = false

[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"

[stores.blob]
local_path = "stores/blob"
"#,
    )
    .expect("write test daemon config");
    config_path
}

fn write_enabled_repo_local_policy(repo_root: &Path) {
    std::fs::write(
        repo_root.join(REPO_POLICY_LOCAL_FILE_NAME),
        r#"[capture]
enabled = true
strategy = "manual-commit"
"#,
    )
    .expect("write repo-local policy");
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_HooksDoNotAutostartWatcher() {
    let parsed = Cli::try_parse_from(["bitloops", "hooks", "git", "post-commit"])
        .expect("hooks command should parse");
    let Some(command) = parsed.command else {
        panic!("expected hooks command");
    };

    assert!(
        !should_attempt_watcher_autostart(&command),
        "hook commands should not attempt DevQL watcher autostart"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_BuildMetadataFallbackValuesAreNonEmpty() {
    assert!(!build_version().is_empty());
    assert!(!build_commit().is_empty());
    assert!(!build_target().is_empty());
    assert!(!build_date().is_empty());
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_Metadata() {
    let root = Cli::command();
    assert_eq!(root.get_name(), ROOT_NAME);
    assert_eq!(
        root.get_about().map(|about| about.to_string()),
        Some(ROOT_SHORT_ABOUT.to_string())
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_LongHelpIncludesGettingStartedAndAccessibility() {
    let help = render_long_help(Cli::command());

    assert!(
        help.contains("Getting Started:"),
        "long help should include getting-started guidance"
    );
    assert!(
        help.contains("bitloops init"),
        "long help should include the init command in getting-started guidance"
    );
    assert!(
        help.contains("Environment Variables:"),
        "long help should include accessibility environment details"
    );
    assert!(
        help.contains("ACCESSIBLE"),
        "long help should document ACCESSIBLE env var"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CompletionDefaultHidden() {
    let root = Cli::command();
    let completion = root
        .get_subcommands()
        .find(|sub| sub.get_name() == "completion")
        .expect("completion command should exist");

    assert!(
        completion.is_hide_set(),
        "completion command should be hidden"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_SubcommandWiring() {
    let root = Cli::command();
    let names = root
        .get_subcommands()
        .map(|sub| sub.get_name())
        .collect::<Vec<_>>();

    for expected in [
        "daemon",
        "start",
        "stop",
        "status",
        "restart",
        "checkpoints",
        "rewind",
        "resume",
        "clean",
        "reset",
        "enable",
        "disable",
        "uninstall",
        "dashboard",
        "hooks",
        "version",
        "explain",
        "debug",
        "devql",
        "doctor",
        "__send_analytics",
        "completion",
        "curl-bash-post-install",
        "help",
    ] {
        assert!(
            names.contains(&expected),
            "root command should include subcommand {expected}"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_HiddenVisibilityForInternalCommands() {
    let root = Cli::command();
    for name in [
        "hooks",
        "debug",
        "__daemon-process",
        "__daemon-supervisor",
        "__send_analytics",
        "completion",
        "curl-bash-post-install",
    ] {
        let cmd = find_subcommand(&root, name);
        assert!(
            cmd.is_hide_set(),
            "{name} should stay hidden in root command wiring"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_HasHiddenInChain_MixedValues() {
    assert!(has_hidden_in_chain(&[false, true, false]));
    assert!(!has_hidden_in_chain(&[false, false, false]));
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_WatcherAutostartMatrix() {
    let cases = [
        (["bitloops", "clean"].as_slice(), false),
        (["bitloops", "disable"].as_slice(), false),
        (["bitloops", "uninstall", "--full"].as_slice(), false),
        (["bitloops", "help"].as_slice(), false),
        (["bitloops", "version"].as_slice(), false),
        (["bitloops", "status"].as_slice(), false),
        (["bitloops", "dashboard"].as_slice(), false),
        (["bitloops", "doctor"].as_slice(), false),
        (["bitloops", "resume", "main"].as_slice(), false),
        (
            ["bitloops", "devql", "query", "repo(\"bitloops\")"].as_slice(),
            true,
        ),
        (["bitloops", "devql", "schema"].as_slice(), false),
    ];

    for (argv, expected) in cases {
        let parsed = Cli::try_parse_from(argv).expect("command should parse");
        let Some(command) = parsed.command else {
            panic!("expected subcommand for {:?}", argv);
        };

        assert_eq!(
            should_attempt_watcher_autostart(&command),
            expected,
            "unexpected watcher autostart decision for {:?}",
            argv
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ResolveWatcherAutostartConfigRoot_UsesDaemonOverrideRoot() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("bitloops");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_enabled_repo_local_policy(&repo_root);

    let config_path = write_test_daemon_config(dir.path());
    let config_path_string = config_path.to_string_lossy().to_string();

    with_env_var(
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(config_path_string.as_str()),
        || {
            let config_root = resolve_watcher_autostart_config_root(&repo_root, &repo_root)
                .expect("watcher autostart should resolve daemon config root");
            let expected_root = dir
                .path()
                .canonicalize()
                .unwrap_or_else(|_| dir.path().to_path_buf());

            assert_eq!(config_root, expected_root);
            assert_ne!(config_root, repo_root);
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ResolveWatcherAutostartConfigRoot_DoesNotFallBackToRepoRoot() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("bitloops");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    write_enabled_repo_local_policy(&repo_root);

    with_env_var(ENV_DAEMON_CONFIG_PATH_OVERRIDE, None, || {
        let config_root = resolve_watcher_autostart_config_root(&repo_root, &repo_root);
        assert!(
            config_root.is_none(),
            "watcher autostart should fail closed when no daemon config root is available"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CustomHelpCommand_HasHiddenTreeFlag() {
    let root = Cli::command();
    let help_cmd = find_subcommand(&root, "help");
    let tree_flag = help_cmd
        .get_arguments()
        .find(|arg| arg.get_long() == Some("tree"))
        .expect("help command should expose hidden --tree/-t flag");

    assert!(
        tree_flag.is_hide_set(),
        "tree flag should be hidden from normal help output"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CustomHelpCommand_TreeOutputSkipsHiddenCommands() {
    let tree = render_custom_help(&[], true);

    assert!(
        tree.lines().next() == Some("bitloops"),
        "tree output should start with root command name"
    );
    assert!(
        tree.contains("resume"),
        "tree output should include visible subcommands"
    );
    assert!(
        !tree.contains("hooks"),
        "tree output should exclude hidden commands"
    );
    assert!(
        !tree.contains("__send_analytics"),
        "tree output should exclude hidden internal commands"
    );
    assert!(
        !tree.contains("\n├── help") && !tree.contains("\n└── help"),
        "tree output should exclude help command itself"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CustomHelpCommand_FallbackToRootOnUnknownTarget() {
    // Stabilize help text: clap can embed ANSI in long help when NO_COLOR is unset.
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let help = render_custom_help(&["not-a-real-command"], false);
        assert!(
            help.contains("Bitloops CLI"),
            "unknown help target should fallback to root command help"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ParseWithoutSubcommand_ForRunEDefaultHelp() {
    let parsed =
        Cli::try_parse_from(["bitloops"]).expect("root invocation without subcommand should parse");
    assert!(
        parsed.command.is_none(),
        "root invocation should map to empty command for RunE-style help behavior"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ParseConnectionStatusFlag() {
    let parsed = Cli::try_parse_from(["bitloops", "--connection-status"])
        .expect("root invocation with --connection-status should parse");
    assert!(parsed.connection_status);
    assert!(
        parsed.command.is_none(),
        "--connection-status should work without subcommands"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ParseVersionFlag() {
    let parsed = Cli::try_parse_from(["bitloops", "--version"])
        .expect("root invocation with --version should parse");
    assert!(parsed.version, "--version should set the version flag");
    assert!(
        !parsed.check,
        "--check should remain false when not explicitly provided"
    );
    assert!(
        parsed.command.is_none(),
        "--version should work without subcommands"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ParseVersionFlagWithCheck() {
    let parsed = Cli::try_parse_from(["bitloops", "--version", "--check"])
        .expect("root invocation with --version --check should parse");
    assert!(parsed.version, "--version should be set");
    assert!(parsed.check, "--check should be set");
    assert!(
        parsed.command.is_none(),
        "--version --check should not require a subcommand"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CheckFlagRequiresVersionFlag() {
    assert!(
        Cli::try_parse_from(["bitloops", "--check"]).is_err(),
        "--check should be rejected without --version"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_VersionSubcommandSupportsCheckFlag() {
    let parsed = Cli::try_parse_from(["bitloops", "version", "--check"])
        .expect("version subcommand should parse with --check");
    let Some(Commands::Version(args)) = parsed.command else {
        panic!("expected version subcommand");
    };
    assert!(args.check, "version subcommand should capture --check");
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_ConnectionStatusFlagCannotBeCombinedWithSubcommand() {
    let parsed = Cli::try_parse_from(["bitloops", "--connection-status", "status"])
        .expect("parser should allow global flag before subcommands");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime.block_on(super::run(parsed));
    assert!(
        result.is_err(),
        "runtime should reject --connection-status combined with subcommands"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_DashboardDefaults() {
    let parsed =
        Cli::try_parse_from(["bitloops", "dashboard"]).expect("dashboard invocation should parse");

    let Some(Commands::Dashboard(args)) = parsed.command else {
        panic!("expected dashboard command");
    };

    let _ = args;
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_RunWithoutSubcommand_ReturnsOk() {
    let parsed =
        Cli::try_parse_from(["bitloops"]).expect("root invocation without subcommand should parse");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime.block_on(super::run(parsed));
    assert!(
        result.is_ok(),
        "root invocation should return Ok after printing help"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CompletionCommandOutputsScripts() {
    let parsed = Cli::try_parse_from(["bitloops", "completion", "zsh"])
        .expect("completion subcommand should parse");
    assert!(
        matches!(parsed.command, Some(Commands::Completion(_))),
        "completion parser should map to Commands::Completion"
    );

    let mut bash = Vec::new();
    write_completion(&mut bash, CompletionShell::Bash).expect("bash completion should render");
    let bash = String::from_utf8(bash).expect("bash completion utf8");
    assert!(
        bash.contains("bitloops"),
        "bash completion should mention binary name"
    );
    assert!(
        bash.contains("complete"),
        "bash completion should contain completion directives"
    );

    let mut zsh = Vec::new();
    write_completion(&mut zsh, CompletionShell::Zsh).expect("zsh completion should render");
    let zsh = String::from_utf8(zsh).expect("zsh completion utf8");
    assert!(
        zsh.contains("bitloops"),
        "zsh completion should mention binary name"
    );
    assert!(
        zsh.contains("compdef"),
        "zsh completion should contain compdef header"
    );

    let mut fish = Vec::new();
    write_completion(&mut fish, CompletionShell::Fish).expect("fish completion should render");
    let fish = String::from_utf8(fish).expect("fish completion utf8");
    assert!(
        fish.contains("bitloops"),
        "fish completion should mention binary name"
    );
    assert!(
        fish.contains("complete -c bitloops"),
        "fish completion should contain fish completion entries"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CurlBashPostInstall_WiresShellCompletionForSupportedShell() {
    let home = TempDir::new().expect("temp home should be created");
    let home_value = home.path().display().to_string();

    with_env_vars(
        &[
            ("HOME", Some(home_value.as_str())),
            ("SHELL", Some("/bin/zsh")),
        ],
        || {
            let mut out = Vec::new();
            let mut input = Cursor::new(b"yes\n".to_vec());
            let result = run_curl_bash_post_install_command_with_io(&mut out, &mut input);
            assert!(result.is_ok(), "post-install command should succeed");

            let rc_file = home.path().join(".zshrc");
            let content = std::fs::read_to_string(&rc_file)
                .unwrap_or_else(|e| panic!("expected {} to exist: {e}", rc_file.display()));
            assert!(
                content.contains(super::enable::SHELL_COMPLETION_COMMENT),
                "rc file should contain shell completion comment"
            );
            assert!(
                content.contains("bitloops completion zsh"),
                "rc file should contain zsh completion command"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CurlBashPostInstall_SupportedShellNoSkipsAppend() {
    let home = TempDir::new().expect("temp home should be created");
    let home_value = home.path().display().to_string();
    with_env_vars(
        &[
            ("HOME", Some(home_value.as_str())),
            ("SHELL", Some("/bin/zsh")),
        ],
        || {
            let mut out = Vec::new();
            let mut input = Cursor::new(b"no\n".to_vec());
            let result = run_curl_bash_post_install_command_with_io(&mut out, &mut input);
            assert!(result.is_ok(), "post-install command should succeed");

            assert!(
                !home.path().join(".zshrc").exists(),
                "answering no should not create shell rc file"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_CurlBashPostInstall_UnsupportedShellIsBestEffort() {
    let home = TempDir::new().expect("temp home should be created");
    let home_value = home.path().display().to_string();
    with_env_vars(
        &[
            ("HOME", Some(home_value.as_str())),
            ("SHELL", Some("/bin/tcsh")),
        ],
        || {
            let mut out = Vec::new();
            let mut input = Cursor::new(Vec::<u8>::new());
            let result = run_curl_bash_post_install_command_with_io(&mut out, &mut input);
            assert!(
                result.is_ok(),
                "unsupported shell should not fail hidden post-install command"
            );

            assert!(
                !home.path().join(".zshrc").exists()
                    && !home.path().join(".bashrc").exists()
                    && !home.path().join(".bash_profile").exists()
                    && !home
                        .path()
                        .join(".config")
                        .join("fish")
                        .join("config.fish")
                        .exists(),
                "unsupported shell should not create shell completion rc files"
            );
        },
    );
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_VersionOutput() {
    with_env_vars(&[("NO_COLOR", Some("1"))], || {
        let mut out = Vec::new();
        write_version(
            &mut out,
            "0.0.10",
            "8f3c9c2abcdef",
            "aarch64-apple-darwin",
            "2026-03-11",
        )
        .expect("version output should render");

        let rendered = String::from_utf8(out).expect("version output utf8");
        assert!(
            !rendered.contains("\u{1b}["),
            "NO_COLOR should disable ANSI colour output"
        );
        assert!(
            rendered.contains(&bitloops_wordmark()),
            "version output should include the brand mark"
        );
        assert!(
            rendered.contains("Bitloops CLI v0.0.10\n"),
            "version output should include the formatted version header"
        );
        assert!(
            rendered.contains("───────────────────\n"),
            "version output should include the divider line"
        );
        assert!(
            rendered.contains("commit: 8f3c9c2\n"),
            "version output should print the short commit hash"
        );
        assert!(
            rendered.contains("target: aarch64-apple-darwin\n"),
            "version output should include the full target triple"
        );
        assert!(
            rendered.contains("built: 2026-03-11\n"),
            "version output should include build date"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestRootCommand_SendAnalytics_ExactArgsValidation() {
    assert!(
        Cli::try_parse_from(["bitloops", "__send_analytics"]).is_err(),
        "__send_analytics should reject missing payload"
    );
    assert!(
        Cli::try_parse_from(["bitloops", "__send_analytics", "{\"event\":1}", "extra"]).is_err(),
        "__send_analytics should reject extra positional args"
    );

    let parsed = Cli::try_parse_from(["bitloops", "__send_analytics", "{\"event\":1}"])
        .expect("__send_analytics should accept exactly one payload argument");
    match parsed.command {
        Some(super::Commands::SendAnalytics(args)) => {
            assert_eq!(args.payload, "{\"event\":1}");
        }
        _ => panic!("expected SendAnalytics subcommand"),
    }
}

#[test]
#[allow(non_snake_case)]
fn TestPersistentPostRun_SkipsHiddenParent() {
    let root = Cli::command();

    // Find the leaf command: bitloops hooks git post-commit.
    // This exercises the real command tree where "hooks" is hidden but descendants are not.
    let hooks = find_subcommand(&root, "hooks");
    let git = find_subcommand(hooks, "git");
    let post_commit = find_subcommand(git, "post-commit");

    assert!(
        !post_commit.is_hide_set(),
        "leaf command should not be hidden itself - the test validates parent-chain detection"
    );

    // Walk the parent chain (excluding root and leaf) and require at least one hidden ancestor.
    let hidden_parent_chain = vec![git.is_hide_set(), hooks.is_hide_set()];
    assert!(
        has_hidden_in_chain(&hidden_parent_chain),
        "expected at least one hidden ancestor between the leaf and root"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestPersistentPostRun_ParentHiddenWalk() {
    struct TestCase {
        name: &'static str,
        hidden_chain: Vec<bool>, // leaf -> ... -> root
        want_hidden: bool,
    }

    let tests = vec![
        TestCase {
            name: "leaf hidden",
            hidden_chain: vec![true, false],
            want_hidden: true,
        },
        TestCase {
            name: "parent hidden, leaf visible",
            hidden_chain: vec![false, true, false],
            want_hidden: true,
        },
        TestCase {
            name: "grandparent hidden, leaf visible",
            hidden_chain: vec![false, false, true, false],
            want_hidden: true,
        },
        TestCase {
            name: "no hidden ancestor",
            hidden_chain: vec![false, false, false],
            want_hidden: false,
        },
    ];

    for tt in tests {
        let got_hidden = has_hidden_in_chain(&tt.hidden_chain);
        assert_eq!(
            got_hidden, tt.want_hidden,
            "case {}: isHidden = {}, want {}",
            tt.name, got_hidden, tt.want_hidden
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_StartAliasCollapsesToCanonicalDaemonStart() {
    let parsed =
        Cli::try_parse_from(["bitloops", "start", "-d"]).expect("start alias should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops daemon start");
    assert_eq!(action.surface, "cli");
    assert_eq!(
        action.properties.get("flags"),
        Some(&Value::Array(vec![Value::String("detached".to_string())]))
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_HiddenInternalCommandsDoNotEmit() {
    let analytics = Cli::try_parse_from(["bitloops", "__send_analytics", "{}"])
        .expect("internal analytics command should parse");
    let analytics_command = analytics.command.as_ref().expect("command");
    assert!(
        telemetry_action_for_command(analytics_command).is_none(),
        "internal analytics command should not emit telemetry"
    );

    let completion =
        Cli::try_parse_from(["bitloops", "completion", "bash"]).expect("completion should parse");
    let completion_command = completion.command.as_ref().expect("command");
    assert!(
        telemetry_action_for_command(completion_command).is_none(),
        "hidden completion command should not emit telemetry"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DaemonStartCanonicalCommandUsesSameEventName() {
    let parsed = Cli::try_parse_from(["bitloops", "daemon", "start", "--until-stopped"])
        .expect("daemon start should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops daemon start");
    assert_eq!(action.surface, "cli");
    assert_eq!(
        action.properties.get("flags"),
        Some(&Value::Array(vec![Value::String(
            "until_stopped".to_string()
        )]))
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DevqlTasksEnqueueSyncTracksSafeProperties() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "tasks",
        "enqueue",
        "--kind",
        "sync",
        "--paths",
        "a,b",
        "--status",
    ])
    .expect("devql tasks enqueue should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops devql tasks enqueue");
    assert_eq!(
        action.properties.get("task_kind").and_then(Value::as_str),
        Some("sync")
    );
    assert_eq!(
        action.properties.get("sync_mode").and_then(Value::as_str),
        Some("paths")
    );
    assert_eq!(
        action.properties.get("paths_count").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        action
            .properties
            .get("status_follow")
            .and_then(Value::as_bool),
        Some(true)
    );
    let rendered = serde_json::to_string(&action.properties).expect("serialize properties");
    assert!(
        !rendered.contains("\"a\"") && !rendered.contains("\"b\""),
        "telemetry should not include raw path values"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DevqlQueryTracksRawGraphqlModeWithoutQueryText() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "query",
        "--graphql",
        "query DashboardRepos { repositories { name } }",
    ])
    .expect("raw GraphQL query should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops devql query");
    assert_eq!(
        action.properties.get("query_mode").and_then(Value::as_str),
        Some("raw_graphql")
    );
    assert!(
        !action.properties.contains_key("stage_sequence"),
        "raw GraphQL mode should not capture stage sequence"
    );
    let rendered = serde_json::to_string(&action.properties).expect("serialize properties");
    assert!(
        !rendered.contains("DashboardRepos") && !rendered.contains("repositories"),
        "telemetry should not include raw GraphQL query text"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DevqlQueryTracksDslStageSequence() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "devql",
        "query",
        "repo(\"x\")->artefacts()->deps()->limit(5)",
    ])
    .expect("devql query should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops devql query");
    assert_eq!(
        action.properties.get("query_mode").and_then(Value::as_str),
        Some("dsl")
    );
    let expected_sequence = vec![
        Value::String("repo".to_string()),
        Value::String("artefacts".to_string()),
        Value::String("deps".to_string()),
        Value::String("limit".to_string()),
    ];
    assert_eq!(
        action
            .properties
            .get("stage_sequence")
            .and_then(Value::as_array),
        Some(&expected_sequence)
    );
    let rendered = serde_json::to_string(&action.properties).expect("serialize properties");
    assert!(
        !rendered.contains("\"x\"") && !rendered.contains("limit(5)"),
        "telemetry should not include raw DevQL literals"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DevqlSchemaTracksModeAndOutput() {
    let parsed = Cli::try_parse_from(["bitloops", "devql", "schema", "--global", "--human"])
        .expect("devql schema should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops devql schema");
    assert_eq!(
        action.properties.get("schema_mode").and_then(Value::as_str),
        Some("global")
    );
    assert_eq!(
        action.properties.get("output_mode").and_then(Value::as_str),
        Some("human")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestTelemetryAction_DevqlSchemaDefaultsToMinifiedSlim() {
    let parsed =
        Cli::try_parse_from(["bitloops", "devql", "schema"]).expect("devql schema should parse");
    let command = parsed.command.as_ref().expect("command");
    let action = telemetry_action_for_command(command).expect("telemetry action");

    assert_eq!(action.event, "bitloops devql schema");
    assert_eq!(
        action.properties.get("schema_mode").and_then(Value::as_str),
        Some("slim")
    );
    assert_eq!(
        action.properties.get("output_mode").and_then(Value::as_str),
        Some("minified")
    );
}
