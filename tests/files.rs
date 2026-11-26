//! `files` contracts: payload layout, cutoffs, omissions, selection, and notebooks.

mod common;

use common::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// The environment every `files` test runs with: empty PATH and `tmp/home` as HOME.
fn base_env(tmp: &Path) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    env(&[
        ("PATH", ""),
        ("HOME", home.to_str().unwrap()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
    ])
}

/// Runs `focal files <args>`, or `focal files --all` when `args` is empty.
fn run(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut full = vec!["files"];
    if args.is_empty() {
        full.push("--all");
    } else {
        full.extend_from_slice(args);
    }
    focal(cwd, env, &full, None)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

fn mixed_files(in_repo: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    if in_repo {
        git(&root, &["init", "-q", "-b", "main"]);
    }
    let cwd = root.join("src");
    fs::create_dir(&cwd).unwrap();
    write(&cwd, "notes.md", "keep\n```\ncode\n```\n\n");
    write(&cwd, ".empty", "");
    write(&cwd, ".ignore", "ignored.txt\n");
    write(&cwd, "ignored.txt", "NEVER INCLUDE");
    let result = run(&cwd, &env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    let parts: Vec<&str> = payload.split("\n---\n\n").collect();
    assert_eq!(parts.len(), 2, "{payload}");
    let (header, mut body) = (parts[0], parts[1].to_owned());
    assert!(header.starts_with("<context>\n> [!NOTE]\n"), "{header}");
    let resolved = fs::canonicalize(&cwd).unwrap();
    assert!(
        header.contains(&format!("PWD: {}", resolved.display())),
        "{header}"
    );
    if in_repo {
        assert!(
            header.ends_with("Git Branch: main\nGit Commit: none\n"),
            "{header}"
        );
    } else {
        assert!(!header.contains("Git Branch:"), "{header}");
    }
    let blocks = [
        "# notes.md\n````md\nkeep\n```\ncode\n```\n````\n\n",
        "# .empty\n```empty\n[empty file]\n```\n\n",
        "# .ignore\n```ignore\nignored.txt\n```\n\n",
    ];
    for block in blocks {
        assert_eq!(body.matches(block).count(), 1, "{block:?} in {body:?}");
        body = body.replace(block, "");
    }
    assert_eq!(body, "\n</context>\n");
    let stderr = text(&result.stderr);
    let tokens = payload.chars().count() / 4;
    assert!(
        stderr.contains(&format!("done: Copied context (~{tokens} tokens)")),
        "{stderr}"
    );
    assert!(
        stderr.ends_with("Added fully: 2 files\nAdded empty: 1 files\n"),
        "{stderr}"
    );
}

#[test]
fn mixed_files_outside_repo() {
    mixed_files(false);
}

#[test]
fn mixed_files_in_repo() {
    mixed_files(true);
}

fn cutoffs(data: &[u8], marker: Option<&str>) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let cwd = tmp.path().join("src");
    fs::create_dir(&cwd).unwrap();
    write(&cwd, "long.txt", data);
    let result = run(&cwd, &env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    match marker {
        Some(marker) => {
            let expected = format!("...[file truncated: showing first {marker}]");
            assert!(contains(&result.stdout, expected.as_bytes()));
            assert!(contains(&result.stderr, b"Truncated (size limit): 1 files"));
        }
        None => {
            assert!(!contains(&result.stdout, b"...[file truncated"));
            assert!(contains(&result.stdout, data));
        }
    }
}

#[test]
fn cutoffs_bytes() {
    cutoffs(&b"x".repeat(100_001), Some("100000 of 100001 bytes"));
}

#[test]
fn cutoffs_lines() {
    cutoffs(&b"x\n".repeat(1_501), Some("1500 of 1501 lines"));
}

#[test]
fn cutoffs_at_limit() {
    cutoffs(&b"x".repeat(100_000), None);
}

#[test]
fn empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let cwd = tmp.path().join("empty");
    fs::create_dir(&cwd).unwrap();
    let result = run(&cwd, &env, &[]);
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    assert!(contains(&result.stderr, b"No files selected."));
}

#[test]
fn committed_repo_and_omissions() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    git(&cwd, &["init", "-q", "-b", "main"]);
    write(&cwd, ".gitignore", "secret.txt\n");
    write(&cwd, "secret.txt", "PRIVATE SENTINEL");
    git(&cwd, &["add", ".gitignore"]);
    git(&cwd, &["commit", "-qm", "fixture"]);
    let head = git(&cwd, &["rev-parse", "--short", "HEAD"]);
    write(&cwd, "bundle.MIN.JS", "MINIFIED SENTINEL");
    write(&cwd, "unknown", b"\0\0\0\0\xff\xfe\0\0");
    let result = run(&cwd, &env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let out = &result.stdout;
    assert!(contains(out, format!("Git Commit: {head}\n").as_bytes()));
    assert!(!contains(out, b"SENTINEL"));
    assert!(contains(out, b"[asset/noise file omitted: bundle.MIN.JS]"));
    assert!(contains(out, b"[binary file omitted: unknown]"));
    assert!(contains(&result.stderr, b"Skipped (noise/binary): 2 files"));
    assert!(contains(
        &result.stderr,
        b"done: Copied .gitignore to clipboard"
    ));
}

#[test]
fn direct_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path().join("src");
    fs::create_dir(&root).unwrap();
    write(&root, "a.txt", "A");
    write(&root, "b.txt", "B");
    write(&root, ".fdignore", "b.txt\n");
    let result = run(tmp.path(), &env, &["src/b.txt", "src/", "src/a.txt"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(count(&result.stdout, b"# src/b.txt\n"), 1);
    assert_eq!(count(&result.stdout, b"# src/a.txt\n"), 1);
    assert!(
        find(&result.stdout, b"# src/b.txt\n").unwrap()
            < find(&result.stdout, b"# src/a.txt\n").unwrap()
    );
    let result = run(tmp.path(), &env, &["src"]);
    assert!(!result.status.success());
    assert!(contains(&result.stderr, b"trailing slash"));
    assert!(result.stdout.is_empty());
}

#[test]
fn walk_uses_fd_ignore_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let config = tmp.path().join("config");
    let env = with(
        &base_env(tmp.path()),
        &[("XDG_CONFIG_HOME", config.to_str().unwrap())],
    );
    write(&config, "fd/ignore", "global.txt\n");
    write(&cwd, ".fdignore", "local.txt\n");
    fs::create_dir(cwd.join(".git")).unwrap();
    for filename in [".git/private", "local.txt", "global.txt"] {
        write(&cwd, filename, "EXCLUDED");
    }
    write(&cwd, "visible", "included");
    let result = run(&cwd, &env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(!contains(&result.stdout, b"EXCLUDED"));
    assert!(contains(&result.stdout, b"# visible\n"));
}

#[test]
fn detached_head() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["commit", "-qm", "root", "--allow-empty"]);
    git(root, &["checkout", "-q", "--detach"]);
    write(root, "sample", "hello");
    let sha = git(root, &["rev-parse", "--short", "HEAD"]);
    let result = run(root, &env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(contains(
        &result.stdout,
        format!("Git Branch: \nGit Commit: {sha}\n").as_bytes()
    ));
}

#[test]
fn notebook_rendering() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let long = "é".repeat(4000);
    let payload = json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {"kernelspec": {"language": "Julia", "display_name": "Julia 1"}},
        "cells": [
            {"cell_type": "markdown", "source": ["Example\n", "```"]},
            {
                "cell_type": "code",
                "source": "answer()",
                "execution_count": 0,
                "outputs": [
                    {
                        "output_type": "stream",
                        "name": "stderr",
                        "text": ["\x1b[31mé\x1b[0m", long],
                    },
                    {
                        "output_type": "display_data",
                        "data": {"text/plain": "fallback", "image/png": "unused"},
                    },
                    {
                        "output_type": "execute_result",
                        "data": {"text/html": "<p>html</p>", "text/latex": "unused"},
                    },
                    {
                        "output_type": "display_data",
                        "data": {"application/json": {"z": "é", "a": true}},
                    },
                    {
                        "output_type": "error",
                        "ename": "E",
                        "evalue": "failed",
                        "traceback": ["\x1b[32mline\x1b[0m"],
                    },
                ],
            },
            {"cell_type": "code", "source": " \n"},
            {"cell_type": "raw", "metadata": {"format": "latex"}, "source": "raw"},
        ],
    });
    write(
        tmp.path(),
        "demo.IPYNB",
        serde_json::to_string(&payload).unwrap(),
    );
    let result = run(tmp.path(), &env, &["demo.IPYNB"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let stdout = text(&result.stdout);
    let document = stdout.split("\n---\n\n").nth(1).unwrap();
    let expected = format!(
        "# Notebook: demo.IPYNB\n\n## Notebook Metadata\n\n\
         - Kernel: Julia 1\n- Language: julia\n- Format: nbformat 4.5\n\
         - Cells: 4 total (2 code, 1 markdown, 1 raw)\n\n\
         ## Markdown cell 1\n\n````markdown\nExample\n```\n````\n\n\
         ## Code cell 2 [execution: 0]\n\n```julia\nanswer()\n```\n\n\
         ### Output\n\n```text\n[stderr]\n{long}\n...[output truncated]\n```\n\n\
         ### Output\n\n[image output omitted]\n\n\
         ### Output\n\n```text\nhtml\n```\n\n\
         ### Output\n\n```json\n{{\n  \"z\": \"\\u00e9\",\n  \"a\": true\n}}\n```\n\n\
         ### Output\n\n```text\n[error]\nE: failed\nline\n```\n\n\
         ## Raw cell 4\n\n```latex\nraw\n```\n\n\n</context>\n"
    );
    assert_eq!(document, expected);
}

fn empty_and_malformed_notebooks(source: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    write(tmp.path(), "invalid.ipynb", source);
    write(tmp.path(), "keep.txt", "still here");
    let result = run(tmp.path(), &env, &["invalid.ipynb", "keep.txt"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(contains(&result.stdout, b"# keep.txt\n"));
    if source.is_empty() {
        assert!(contains(&result.stdout, b"\n[empty file]\n\n"));
    } else {
        assert!(contains(
            &result.stdout,
            b"# Notebook: invalid.ipynb\n\n[Error parsing notebook:"
        ));
    }
}

#[test]
fn empty_and_malformed_notebooks_empty() {
    empty_and_malformed_notebooks("");
}

#[test]
fn empty_and_malformed_notebooks_list() {
    empty_and_malformed_notebooks("[]");
}

#[test]
fn empty_and_malformed_notebooks_truncated() {
    empty_and_malformed_notebooks("{");
}

/// A repository in `tmp` with staged deletions, renames, edits, and additions, unstaged
/// edits, and an untracked file.
fn changes(tmp: &Path) -> PathBuf {
    let root = tmp.to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    for name in ["gone", "moved", "edited", "sub/inner", "sub/clean"] {
        write(&root, name, format!("{name}\n"));
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "init"]);
    git(&root, &["rm", "-q", "gone"]);
    git(&root, &["mv", "moved", "renamed"]);
    write(&root, "edited", "staged\n");
    write(&root, "new", "new\n");
    git(&root, &["add", "edited", "new"]);
    write(&root, "edited", "staged\nthen edited\n");
    write(&root, "sub/inner", "unstaged\n");
    write(&root, "untracked", "u\n");
    root
}

fn headings(result: &Output) -> Vec<String> {
    text(&result.stdout)
        .lines()
        .filter(|line| line.starts_with("# ") && *line != "# Workspace Context")
        .map(str::to_owned)
        .collect()
}

#[test]
fn staged_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let changes = changes(tmp.path());
    let result = run(&changes, &env, &["--staged"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(headings(&result), ["# edited", "# new", "# renamed"]);
    assert!(contains(
        &result.stdout,
        b"# edited\n```\nstaged\nthen edited\n```\n"
    ));
    assert!(
        result
            .stderr
            .starts_with(b"info: Discovering staged files...\n")
    );
}

#[test]
fn staged_unborn_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    write(tmp.path(), "first", "1\n");
    git(tmp.path(), &["add", "first"]);
    let result = run(tmp.path(), &env, &["-s"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(headings(&result), ["# first"]);
}

fn modified_selection(subdir: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let changes = changes(tmp.path());
    let cwd = if subdir {
        changes.join("sub")
    } else {
        changes.clone()
    };
    let result = run(&cwd, &env, &["--modified"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let expected: &[&str] = if subdir {
        &["# inner"]
    } else {
        &["# edited", "# sub/inner"]
    };
    assert_eq!(headings(&result), expected);
    assert!(
        result
            .stderr
            .starts_with(b"info: Discovering modified files...\n")
    );
}

#[test]
fn modified_selection_root() {
    modified_selection(false);
}

#[test]
fn modified_selection_subdir() {
    modified_selection(true);
}

#[test]
fn staged_from_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let changes = changes(tmp.path());
    let result = run(&changes.join("sub"), &env, &["--staged"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(
        headings(&result),
        ["# ../edited", "# ../new", "# ../renamed"]
    );
}

#[test]
fn modified_conflict_listed_once() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "conf", "base\n");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["checkout", "-qb", "other"]);
    write(root, "conf", "theirs\n");
    git(root, &["commit", "-qam", "theirs"]);
    git(root, &["checkout", "-q", "main"]);
    write(root, "conf", "ours\n");
    git(root, &["commit", "-qam", "ours"]);
    let merge = git_output(root, &["merge", "other"]);
    let mut combined = merge.stdout.clone();
    combined.extend_from_slice(&merge.stderr);
    assert!(contains(&combined, b"CONFLICT"));
    let result = run(root, &env, &["-m"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(headings(&result), ["# conf"]);
}
