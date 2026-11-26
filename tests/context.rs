//! Non-interactive `context` contracts.

mod common;

use common::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Output;

fn base_env(tmp: &Path) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    env(&[
        ("PATH", ""),
        ("HOME", home.to_str().unwrap()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_AUTHOR_NAME", "a"),
        ("GIT_AUTHOR_EMAIL", "a@example.com"),
        ("GIT_COMMITTER_NAME", "a"),
        ("GIT_COMMITTER_EMAIL", "a@example.com"),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("FOCAL_OMIT_WORKSPACE", "1"),
    ])
}

fn run(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut full = vec!["context"];
    full.extend_from_slice(args);
    focal(cwd, env, &full, None)
}

/// The top-level `# ` headings of a payload, in order.
fn sections(payload: &str) -> Vec<&str> {
    payload
        .lines()
        .filter(|line| line.starts_with("# "))
        .collect()
}

/// A repository at `tmp/repo` mid-merge with a conflict, a staged rename with an unstaged
/// edit, a staged deletion, a staged and unstaged modification, a type change, untracked
/// files with names Git quotes, and an ignored file.
fn repo(tmp: &Path) -> PathBuf {
    let root = tmp.join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, "Cargo.toml", "[package]\n");
    write(&root, "README.md", "Demo\n");
    write(&root, "big", "one\ntwo\nthree\nfour\nfive\n");
    write(&root, "conf", "base\n");
    write(&root, "del", "d\n");
    write(&root, "mod", "m\n");
    write(&root, "tc", "t\n");
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "init"]);
    git(&root, &["checkout", "-qb", "other"]);
    write(&root, "conf", "theirs\n");
    git(&root, &["commit", "-qam", "theirs"]);
    git(&root, &["checkout", "-q", "main"]);
    write(&root, "conf", "ours\n");
    git(&root, &["commit", "-qam", "ours"]);
    git_output(&root, &["merge", "-q", "other"]);
    git(&root, &["mv", "big", "renamed"]);
    write(&root, "renamed", "one\ntwo\nthree\nfour\nfive\nsix\n");
    git(&root, &["rm", "-q", "del"]);
    write(&root, "mod", "mm\n");
    git(&root, &["add", "mod"]);
    write(&root, "mod", "mmm\n");
    fs::remove_file(root.join("tc")).unwrap();
    symlink("mod", root.join("tc")).unwrap();
    write(&root, "a b", "");
    write(&root, "q\"x", "");
    write(&root, "é", "");
    write(&root, "untracked/nested/f", "");
    write(&root, "ignored/z.log", "");
    write(&root, ".gitignore", "*.log\n");
    root
}

fn core_git_info(subdir: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let repo = repo(tmp.path());
    let cwd = if subdir {
        repo.join("untracked")
    } else {
        repo.clone()
    };
    let result = run(&cwd, &env, &["--core"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    let sha = git(&repo, &["rev-parse", "--short", "HEAD"]);
    let expected = format!(
        "# Git Info\n```text\nBranch: main\nCommit: {sha}\n\nModified files:\n\
         UU conf\nD  del\nMM mod\nRM big -> renamed\n T tc\n\
         ?? .gitignore\n?? \"a b\"\n?? \"q\\\"x\"\n?? untracked/\n?? \"\\303\\251\"\n```\n\n"
    );
    assert!(payload.contains(&expected), "{payload}");
    let mut want = vec!["# Repository Tree", "# Git Info"];
    if !subdir {
        want.extend(["# README.md", "# Cargo.toml"]);
    }
    assert_eq!(sections(&payload), want);
    assert!(
        text(&result.stderr).starts_with("done: Context and files copied to clipboard (~"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn core_git_info_root() {
    core_git_info(false);
}

#[test]
fn core_git_info_subdir() {
    core_git_info(true);
}

#[test]
fn core_outside_git_with_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    write(root, "README.md", "readme\n");
    write(root, "pyproject.toml", "[project]\n");
    write(root, "src/b.py", "b = 1\n");
    write(root, "src/a.py", "a = 1\n");
    write(root, "extra.txt", "extra\n");
    let result = run(root, &env, &["-c", "extra.txt", "src/"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    assert!(
        payload.starts_with("<context>\n# Repository Tree\n```text\n"),
        "{payload}"
    );
    assert!(!payload.contains("# Git Info"), "{payload}");
    assert_eq!(
        sections(&payload)[1..],
        [
            "# README.md",
            "# pyproject.toml",
            "# extra.txt",
            "# src/a.py",
            "# src/b.py",
        ]
    );
    assert!(
        payload.contains("# README.md\n```md\nreadme\n```\n\n"),
        "{payload}"
    );
    assert!(payload.ends_with("```\n\n\n</context>\n"), "{payload}");
}

#[test]
fn core_empty_status() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    git(tmp.path(), &["init", "-q", "-b", "trunk"]);
    let result = run(tmp.path(), &env, &["--core"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    assert!(
        payload.ends_with(
            "# Git Info\n```text\nBranch: trunk\nCommit: none\n\nModified files:\n```\n\n\n</context>\n"
        ),
        "{payload}"
    );
}

#[test]
fn missing_path() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let result = run(tmp.path(), &env, &["--core", "absent.txt"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        text(&result.stderr).contains("file not found: absent.txt"),
        "{}",
        text(&result.stderr)
    );
    assert!(result.stdout.is_empty());
}

#[test]
fn all_includes_each_file_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    write(root, "README.md", "root\n");
    write(root, ".hidden", "h\n");
    write(root, "lib/code.rs", "fn main() {}\n");
    let result = run(root, &env, &["--all", "lib/code.rs"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    let found = sections(&payload);
    assert_eq!(found[..2], ["# Repository Tree", "# README.md"]);
    let mut rest = found[2..].to_vec();
    rest.sort_unstable();
    assert_eq!(rest, ["# .hidden", "# lib/code.rs"]);
    assert!(
        result
            .stderr
            .starts_with(b"info: Discovering all valid files...\n"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn all_keeps_nested_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    write(root, "README.md", "root\n");
    write(root, "docs/README.md", "nested\n");
    let result = run(root, &env, &["--all"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    assert_eq!(sections(&payload)[1..], ["# README.md", "# docs/README.md"]);
}

#[test]
fn core_deduplicates_named_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    write(tmp.path(), "README.md", "root\n");
    let result = run(tmp.path(), &env, &["--core", "README.md"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    assert_eq!(sections(&payload)[1..], ["# README.md"]);
}
