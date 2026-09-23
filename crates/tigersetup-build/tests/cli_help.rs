//! How `tiger-setup` answers a person asking for help: `-h`/`--help` is the
//! complete reference, before or after a command; `--help-brief` is the short
//! landing text; there is no `help` command.

use std::process::{Command, Output};

const BUILDER: &str = env!("CARGO_BIN_EXE_tiger-setup");

fn run(args: &[&str]) -> Output {
    Command::new(BUILDER).args(args).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn help_is_the_complete_reference_and_names_no_installed_location() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        let text = stdout(&output);
        assert_eq!(output.status.code(), Some(0), "{flag}: {text}");
        assert!(text.contains("Usage: tiger-setup"), "{text}");
        for command in ["build", "metadata", "inspect", "verify", "winget", "shell"] {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(command)),
                "{command} in {text}"
            );
        }
        assert!(text.contains("--help-brief"), "{text}");
        assert!(text.contains("tiger-setup <command> --help"), "{text}");
        // Reference, not installation-location documentation.
        assert!(!text.contains("Start > TigerSetup"), "{text}");
        assert!(!text.contains("TigerSetup-Help"), "{text}");
        assert!(!text.contains("Help:"), "{text}");
        assert!(!text.contains("tiger-setup help"), "{text}");
    }
}

#[test]
fn a_command_has_its_own_help() {
    let output = run(&["build", "--help"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("Usage: tiger-setup"), "{text}");
    assert!(text.contains("--fast"), "{text}");
}

#[test]
fn there_is_no_help_command() {
    for args in [&["help"][..], &["help", "build"], &["winget", "help"]] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand 'help'"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn the_brief_help_is_short_plain_when_redirected_and_stands_alone() {
    let output = run(&["--help-brief"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(
        text.starts_with(concat!("TigerSetup ", env!("CARGO_PKG_VERSION"))),
        "{text}"
    );
    assert!(
        text.contains("  tiger-setup build TigerSetup.toml"),
        "{text}"
    );
    assert!(!text.contains("Usage:"), "{text}");
    assert!(
        !text.contains('\x1b'),
        "redirected output is plain: {text:?}"
    );
    // Outside TigerSetup Shell there is no PATH line to make.
    assert!(!text.contains("on PATH"), "{text}");
    assert!(text.lines().count() <= 18, "{text}");

    let with_command = run(&["--help-brief", "build", "TigerSetup.toml"]);
    assert_eq!(with_command.status.code(), Some(2));
}
