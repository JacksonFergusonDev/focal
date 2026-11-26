//! Dispatcher contracts: help, the command list, global flags, and unknown commands.

mod common;

use common::*;
use std::path::Path;
use std::process::Output;

const COMMANDS: [&str; 13] = [
    "api",
    "ci-fail",
    "context",
    "diff",
    "files",
    "issues",
    "pr-diff",
    "release-context",
    "search",
    "symbol",
    "tree",
    "web",
    "wip-context",
];

fn run(tmp: &Path, args: &[&str]) -> Output {
    let home = tmp.to_str().unwrap();
    let env = env(&[
        ("PATH", ""),
        ("HOME", home),
        ("NO_COLOR", "1"),
        ("LC_ALL", "C"),
    ]);
    focal(tmp, &env, args, None)
}

fn check_help_menu(args: &[&str]) {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), args);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(
        out.starts_with("Usage: focal [global flags] <command> [args]\n\nGlobal flags:\n"),
        "{out}"
    );
    assert!(
        out.contains("  --repo <url>      Run the command on a remote GitHub repository"),
        "{out}"
    );
    let listed: Vec<&str> = out
        .split("Available commands:\n")
        .nth(1)
        .expect("command list")
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(listed, COMMANDS);
    assert_eq!(text(&result.stderr), "");
}

#[test]
fn help_menu_no_args() {
    check_help_menu(&[]);
}

#[test]
fn help_menu_help() {
    check_help_menu(&["help"]);
}

#[test]
fn help_menu_short_flag() {
    check_help_menu(&["-h"]);
}

#[test]
fn help_menu_long_flag() {
    check_help_menu(&["--help"]);
}

#[test]
fn command_list_for_completion() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &["--commands"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert_eq!(out.lines().collect::<Vec<_>>(), COMMANDS);
}

#[test]
fn unknown_command() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &["frobnicate"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(text(&result.stdout), "");
    assert_eq!(
        text(&result.stderr),
        "error: unknown command 'frobnicate'\n  hint: run 'focal help' for a list of available commands\n"
    );
}

fn check_repo_flag_requires_value(args: &[&str]) {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), args);
    assert_eq!(result.status.code(), Some(1));
    let err = text(&result.stderr);
    assert!(
        err.starts_with("error: --repo requires an argument.\n  hint: "),
        "{err}"
    );
}

#[test]
fn repo_flag_requires_value_global() {
    check_repo_flag_requires_value(&["--repo"]);
}

#[test]
fn repo_flag_requires_value_after_command() {
    check_repo_flag_requires_value(&["tree", "--repo"]);
}

#[test]
fn double_dash_reaches_the_command() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "flags.txt", "use --repo here\n");
    let result = run(tmp.path(), &["search", "--", "--repo"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    assert!(
        text(&result.stdout).contains("./flags.txt:use --repo here"),
        "{}",
        text(&result.stdout)
    );
}

#[test]
fn empty_repo_value_runs_locally() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &["--repo=", "tree"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    assert!(
        text(&result.stdout).starts_with("<context>\n"),
        "{}",
        text(&result.stdout)
    );
}
