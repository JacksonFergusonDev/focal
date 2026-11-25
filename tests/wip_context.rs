//! `wip-context` contracts.
//!
//! Assertions check content, section order, and budgets rather than byte-identical Git
//! output: topology spacing, rename scores, and file-type descriptions may differ.

mod common;

use common::*;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The test environment: a private HOME, a fixed Git identity, and no colour.
fn base_env(tmp: &Path) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    env(&[
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

/// Runs the host `git` with exactly `env` plus the caller's PATH; returns trimmed stdout.
fn g(cwd: &Path, env: &Env, args: &[&str]) -> String {
    let out: Output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        text(&out.stderr)
    );
    text(&out.stdout).trim().to_owned()
}

fn commit(cwd: &Path, env: &Env, message: &str, when: &str) {
    let dated = with(
        env,
        &[("GIT_AUTHOR_DATE", when), ("GIT_COMMITTER_DATE", when)],
    );
    g(cwd, &dated, &["add", "-A"]);
    g(cwd, &dated, &["commit", "-qm", message]);
}

/// Runs `focal wip-context <args>` with an empty PATH.
fn run(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut full = vec!["wip-context"];
    full.extend_from_slice(args);
    focal(cwd, &with(env, &[("PATH", "")]), &full, None)
}

struct Repo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    env: Env,
}

fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    g(&root, &env, &["init", "-q", "-b", "main"]);
    write(&root, "README.md", "readme\n");
    write(&root, "pkg/core.py", "value = 1\n");
    commit(&root, &env, "base", "2001-01-01T00:00:00Z");
    g(&root, &env, &["checkout", "-qb", "feature"]);
    write(&root, "pkg/core.py", "value = 2\n");
    write(&root, ".github/ci.yml", "on: push\n");
    commit(&root, &env, "update core", "2001-01-02T00:00:00Z");
    write(&root, "README.md", "readme\nmore\n");
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(std::iter::repeat_n(0u8, 2000));
    write(&root, "art.png", png);
    commit(&root, &env, "docs and art", "2001-01-03T00:00:00Z");
    Repo {
        _tmp: tmp,
        root,
        env,
    }
}

fn sections(payload: &[u8]) -> Vec<String> {
    let re = Regex::new(r"(?m)^#+ .*$").unwrap();
    re.find_iter(&text(payload))
        .map(|m| m.as_str().to_owned())
        .collect()
}

/// The text after the first `start` and before the following `end`.
fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let rest = text
        .split_once(start)
        .unwrap_or_else(|| panic!("no {start:?} in {text}"))
        .1;
    rest.split(end).next().unwrap()
}

#[test]
fn branch_context() {
    let r = repo();
    write(&r.root, "pkg/core.py", "value = 3\n");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    let base = g(&r.root, &r.env, &["merge-base", "main", "HEAD"])[..7].to_owned();
    let head = g(&r.root, &r.env, &["rev-parse", "--short", "HEAD"]);
    let prefix = format!(
        "<context>\n# WIP Branch Context\n**Base:** `main` ({base}) | **HEAD:** ({head})\n"
    );
    assert!(out.starts_with(&prefix), "{out}");
    assert_eq!(
        sections(&result.stdout),
        [
            "# WIP Branch Context",
            "## 0. Uncommitted Changes",
            "## 1. Branch Topology",
            "## 2. Macroscopic Map",
            "## 3. Microscopic Diffs",
            "## 4. Omitted Files Metadata",
            "### `.github/ci.yml`",
            "### `README.md`",
            "### `pkg/core.py`",
        ]
    );
    assert!(
        out.split_once("## 0. Uncommitted Changes")
            .unwrap()
            .1
            .contains("M pkg/core.py")
    );
    let topology = between(&out, "## 1. Branch Topology\n```text\n", "```");
    let newest = topology.find("docs and art (").unwrap();
    let oldest = topology.find("update core (").unwrap();
    assert!(newest < oldest, "{topology}");
    assert!(
        Regex::new(r"\[[0-9a-f]{7}\] docs and art \(\d+ years ago\)")
            .unwrap()
            .is_match(topology),
        "{topology}"
    );
    assert!(topology.contains("M\tpkg/core.py"));
    assert!(topology.contains("A\t.github/ci.yml"));
    assert!(topology.contains("A\tart.png"));
    let stat = between(&out, "## 2. Macroscopic Map\n```text\n", "```");
    assert!(stat.contains("4 files changed"), "{stat}");
    assert!(
        out.contains("- `art.png` (Size: 2.0K)\n  - Type: "),
        "{out}"
    );
    assert!(out.contains("+value = 2"));
    assert!(!out.split_once("## 3.").unwrap().1.contains("value = 3"));
    assert!(!out.contains("\nindex "));
    assert!(out.ends_with("```\n</context>\n"));
    assert!(
        result
            .stderr
            .starts_with(b"done: Copied WIP branch context (~"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn clean_tree_omits_uncommitted_section() {
    let r = repo();
    let result = run(&r.root, &r.env, &["main"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(!text(&result.stdout).contains("## 0. Uncommitted Changes"));
}

#[test]
fn budget_truncates_and_keeps_fences_closed() {
    let r = repo();
    for n in 0..5 {
        let body: String = (0..200)
            .map(|i| format!("line {i} {}\n", "y".repeat(60)))
            .collect();
        write(&r.root, &format!("pkg/big{n}.py"), body);
    }
    commit(&r.root, &r.env, "big files", "2001-01-04T00:00:00Z");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    let diffs = out.split_once("## 3. Microscopic Diffs").unwrap().1;
    assert!(diffs.contains("...[diff truncated: context limit reached]"));
    let caps = Regex::new(r"diffs for (\d+) remaining files omitted")
        .unwrap()
        .captures(diffs)
        .expect("omitted-files note");
    assert!(caps[1].parse::<usize>().unwrap() >= 2);
    let body = diffs.split_once("## 4. Omitted Files Metadata").unwrap().1;
    let blocks = body.split("\n\n*...diffs for").next().unwrap();
    assert!(blocks.len() <= 30_500, "{}", blocks.len());
    assert_eq!(blocks.matches("```").count() % 2, 0);
}

#[test]
fn remote_default_branch() {
    let r = repo();
    g(&r.root, &r.env, &["branch", "-m", "main", "trunk"]);
    g(
        &r.root,
        &r.env,
        &["update-ref", "refs/remotes/origin/trunk", "trunk"],
    );
    g(
        &r.root,
        &r.env,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("**Base:** `trunk` ("));
}

#[test]
fn remote_only_fallback() {
    let r = repo();
    g(
        &r.root,
        &r.env,
        &["update-ref", "refs/remotes/origin/master", "main"],
    );
    g(&r.root, &r.env, &["branch", "-D", "main"]);
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("**Base:** `origin/master` ("));
}

#[test]
fn missing_base_branch() {
    let r = repo();
    let result = run(&r.root, &r.env, &["nope"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(result.stdout, b"");
    assert_eq!(
        text(&result.stderr),
        "error: specified base branch 'nope' does not exist\n  \
         hint: verify the branch name with 'git branch -a'\n"
    );
}

#[test]
fn no_detectable_base() {
    let r = repo();
    g(&r.root, &r.env, &["branch", "-D", "main"]);
    let result = run(&r.root, &r.env, &[]);
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("could not automatically detect a base branch"));
}

#[test]
fn no_divergence() {
    let r = repo();
    g(&r.root, &r.env, &["checkout", "-q", "main"]);
    let result = run(&r.root, &r.env, &[]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(result.stdout, b"");
    assert_eq!(
        text(&result.stderr),
        "error: no divergent commits found between 'main' and HEAD\n  \
         hint: create commits on this branch or specify a different base branch: \
         focal wip-context <branch>\n"
    );
}

#[test]
fn deleted_noise_file() {
    let r = repo();
    g(&r.root, &r.env, &["checkout", "-q", "main"]);
    write(&r.root, "data.db", "x");
    commit(&r.root, &r.env, "db", "2001-01-01T12:00:00Z");
    g(&r.root, &r.env, &["checkout", "-qb", "drop"]);
    fs::remove_file(r.root.join("data.db")).unwrap();
    write(&r.root, "note.txt", "n\n");
    commit(&r.root, &r.env, "remove db", "2001-01-06T00:00:00Z");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("- `data.db` (Deleted or inaccessible)"));
}

#[test]
fn rename_is_reported_as_rename() {
    let r = repo();
    let lines: String = (0..20).map(|n| format!("line {n}\n")).collect();
    write(&r.root, "pkg/core.py", &lines);
    commit(&r.root, &r.env, "grow core", "2001-01-06T00:00:00Z");
    g(&r.root, &r.env, &["checkout", "-q", "main"]);
    write(&r.root, "pkg/core.py", &lines);
    commit(&r.root, &r.env, "grow core on main", "2001-01-06T00:00:00Z");
    g(&r.root, &r.env, &["checkout", "-q", "feature"]);
    g(&r.root, &r.env, &["merge", "-q", "--no-edit", "main"]);
    g(&r.root, &r.env, &["mv", "pkg/core.py", "pkg/renamed.py"]);
    commit(&r.root, &r.env, "rename", "2001-01-07T00:00:00Z");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(
        text(&result.stdout).contains("rename from pkg/core.py\nrename to pkg/renamed.py"),
        "{}",
        text(&result.stdout)
    );
}
