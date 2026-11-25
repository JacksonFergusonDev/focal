//! `diff` contracts.
//!
//! Assertions check content and section structure rather than byte-identical Git output:
//! stat column padding and rename similarity scores may differ between implementations.

mod common;

use common::*;
use std::fs;
use std::path::Path;
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

/// Runs the host `git` with exactly `env` plus the caller's PATH.
fn git_in(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("run git")
}

fn g(cwd: &Path, env: &Env, args: &[&str]) {
    let out = git_in(cwd, env, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        text(&out.stderr)
    );
}

/// Runs `focal diff <args>` with an empty PATH.
fn run(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut full = vec!["diff"];
    full.extend_from_slice(args);
    focal(cwd, &with(env, &[("PATH", "")]), &full, None)
}

/// The body of a `### name` section, up to the next `### ` heading.
fn section(payload: &[u8], name: &str) -> String {
    let text = text(payload);
    let heading = format!("### {name}\n");
    let start = text
        .find(&heading)
        .unwrap_or_else(|| panic!("no section {name:?} in {text}"))
        + heading.len();
    let end = text[start..]
        .find("\n### ")
        .map_or(text.len(), |i| start + i);
    text[start..end].to_owned()
}

fn stderr_of(out: &Output) -> String {
    text(&out.stderr)
}

struct Repo {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    env: Env,
}

fn repo() -> Repo {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path().to_path_buf();
    g(&root, &env, &["init", "-q", "-b", "main"]);
    write(&root, "main.py", "initial\n");
    write(&root, "uv.lock", "lock v1\n");
    write(&root, "staged.py", "base\n");
    g(&root, &env, &["add", "."]);
    g(&root, &env, &["commit", "-qm", "initial"]);
    Repo {
        _tmp: tmp,
        root,
        env,
    }
}

#[test]
fn unstaged_default_excludes_noise() {
    let r = repo();
    write(&r.root, "main.py", "initial\ncode change\n");
    write(&r.root, "uv.lock", "lock v2\n");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let out = text(&result.stdout);
    assert!(out.starts_with("<context>\n### Status\n```\n"), "{out}");
    assert!(section(&result.stdout, "Status").contains(" M main.py"));
    let summary = section(&result.stdout, "Summary");
    assert!(summary.contains("main.py"));
    assert!(
        summary.contains("1 file changed, 1 insertion(+)"),
        "{summary}"
    );
    let patch = section(&result.stdout, "Diff");
    assert!(
        patch.starts_with("```diff\ndiff --git a/main.py b/main.py\n"),
        "{patch}"
    );
    assert!(patch.contains("+code change\n```\n"));
    assert!(!out.contains("uv.lock"));
    assert!(!out.contains("lock v2"));
    assert!(out.ends_with("```\n\n\n</context>\n"), "{out:?}");
    assert!(
        result
            .stderr
            .starts_with(b"done: Copied diff of working tree (~"),
        "{}",
        stderr_of(&result)
    );
}

#[test]
fn only_noise_reports_no_changes() {
    let r = repo();
    write(&r.root, "uv.lock", "lock v2\n");
    let result = run(&r.root, &r.env, &[]);
    assert_eq!(result.status.code(), Some(0));
    assert_eq!(result.stdout, b"");
    assert_eq!(
        stderr_of(&result),
        "info: No changes detected in working tree.\n"
    );
}

#[test]
fn explicit_path_includes_noise() {
    let r = repo();
    write(&r.root, "uv.lock", "lock v2\n");
    write(&r.root, "main.py", "changed\n");
    let result = run(&r.root, &r.env, &["--", "uv.lock"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    assert!(section(&result.stdout, "Diff").contains("+lock v2"));
    assert!(!text(&result.stdout).contains("main.py"));
    assert!(stderr_of(&result).contains("Copied diff of specified files"));
}

#[test]
fn staged_warns_about_unstaged_and_untracked() {
    let r = repo();
    write(&r.root, "staged.py", "staged\n");
    g(&r.root, &r.env, &["add", "staged.py"]);
    write(&r.root, "main.py", "unstaged\n");
    write(&r.root, "new.txt", "new\n");
    let result = run(&r.root, &r.env, &["--cached"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.contains("+staged"));
    assert!(!patch.contains("unstaged"));
    assert!(!patch.contains("new.txt"));
    let stderr = stderr_of(&result);
    assert!(stderr.contains("warning: You have unstaged changes not included in this diff."));
    assert!(stderr.contains("warning: You have untracked files not included in this diff."));
    assert!(stderr.contains("done: Copied diff of staged changes"));
}

#[test]
fn unstaged_warns_about_staged() {
    let r = repo();
    write(&r.root, "staged.py", "staged\n");
    g(&r.root, &r.env, &["add", "staged.py"]);
    write(&r.root, "main.py", "unstaged\n");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    assert!(!section(&result.stdout, "Diff").contains("+staged"));
    assert!(
        stderr_of(&result).contains("warning: You have staged changes not included in this diff.")
    );
}

#[test]
fn all_includes_untracked_and_deletions() {
    let r = repo();
    write(&r.root, "staged.py", "staged\n");
    g(&r.root, &r.env, &["add", "staged.py"]);
    fs::remove_file(r.root.join("main.py")).unwrap();
    write(&r.root, "with space.txt", "spaced\n");
    write(&r.root, "new.lock", "noise\n");
    let result = run(&r.root, &r.env, &["-a"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.contains("+staged"));
    assert!(patch.contains("-initial"));
    assert!(patch.contains("deleted file mode"));
    assert!(patch.contains("+++ b/with space.txt"));
    assert!(patch.contains("+spaced"));
    assert!(!text(&result.stdout).contains("new.lock"));
    let stderr = stderr_of(&result);
    assert!(!stderr.contains("warning"), "{stderr}");
    assert!(stderr.contains("Copied diff of all changes"));
}

#[test]
fn all_lists_each_patch_once() {
    let r = repo();
    write(&r.root, "main.py", "changed\n");
    write(&r.root, "new.txt", "new\n");
    let result = run(&r.root, &r.env, &["-a"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert_eq!(patch.matches("+++ b/main.py").count(), 1);
    assert_eq!(patch.matches("+++ b/new.txt").count(), 1);
}

#[test]
fn stat_bars_scale_like_git() {
    let r = repo();
    let big: String = (0..200).map(|n| format!("{n}\n")).collect();
    let new: String = (0..20).map(|n| format!("{n}\n")).collect();
    write(&r.root, "big.txt", big);
    write(&r.root, "new.txt", new);
    fs::remove_file(r.root.join("staged.py")).unwrap();
    g(&r.root, &r.env, &["add", "-A"]);
    let result = run(&r.root, &r.env, &["--staged"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let summary = section(&result.stdout, "Summary");
    let rows: std::collections::HashMap<String, String> = summary
        .lines()
        .filter(|line| line.contains('|'))
        .map(|line| {
            let mut parts = line.split('|');
            (
                parts.next().unwrap().trim().to_owned(),
                parts.next().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(!rows["new.txt"].contains('-'), "{summary}");
    assert!(rows["new.txt"].contains('+'), "{summary}");
    assert!(!rows["staged.py"].contains('+'), "{summary}");
    assert!(rows["staged.py"].contains('-'), "{summary}");
}

fn conflicted() -> Repo {
    let r = repo();
    g(&r.root, &r.env, &["checkout", "-qb", "other"]);
    write(&r.root, "main.py", "theirs\n");
    g(&r.root, &r.env, &["commit", "-qam", "theirs"]);
    g(&r.root, &r.env, &["checkout", "-q", "main"]);
    write(&r.root, "main.py", "ours\n");
    g(&r.root, &r.env, &["commit", "-qam", "ours"]);
    // Expected to fail with a conflict.
    let _ = git_in(&r.root, &r.env, &["merge", "-q", "other"]);
    r
}

fn unmerged_path_shows_conflict(mode: &[&str]) {
    let r = conflicted();
    let result = run(&r.root, &r.env, mode);
    assert!(result.status.success(), "{}", stderr_of(&result));
    assert!(section(&result.stdout, "Status").contains("UU main.py"));
    if mode.is_empty() {
        assert!(section(&result.stdout, "Summary").contains("Unmerged"));
    }
    let patch = section(&result.stdout, "Diff");
    assert!(patch.contains("<<<<<<< HEAD"), "{patch}");
    assert!(patch.contains(">>>>>>> other"), "{patch}");
    assert!(!patch.contains("deleted file mode"));
}

#[test]
fn unmerged_path_shows_conflict_default() {
    unmerged_path_shows_conflict(&[]);
}

#[test]
fn unmerged_path_shows_conflict_all() {
    unmerged_path_shows_conflict(&["-a"]);
}

#[test]
fn staged_unmerged_path() {
    let r = conflicted();
    let result = run(&r.root, &r.env, &["--staged"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.contains("* Unmerged path main.py"), "{patch}");
    assert!(!patch.contains("deleted file mode"));
}

#[test]
fn all_on_unborn_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let root = tmp.path();
    g(root, &env, &["init", "-q", "-b", "main"]);
    write(root, "first.py", "one\n");
    g(root, &env, &["add", "first.py"]);
    write(root, "loose.py", "two\n");
    let result = run(root, &env, &["--all"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.contains("+one"));
    assert!(patch.contains("+two"));
}

#[test]
fn fence_outgrows_backticks() {
    let r = repo();
    write(&r.root, "main.py", "initial\n````\n");
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.starts_with("`````diff\n"), "{patch}");
    assert!(patch.contains("+````\n`````\n"), "{patch}");
}

#[test]
fn large_diff_is_not_truncated() {
    let r = repo();
    let body: String = (0..1000)
        .map(|n| format!("line {n:06} {}\n", "x".repeat(40)))
        .collect();
    write(&r.root, "main.py", body);
    let result = run(&r.root, &r.env, &[]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let patch = section(&result.stdout, "Diff");
    assert!(patch.len() > 30_000);
    assert!(patch.contains("+line 000999"));
}

#[test]
fn thick_appends_tree_and_files() {
    let r = repo();
    write(&r.root, "pkg/mod.py", "x = 1\n");
    g(&r.root, &r.env, &["add", "."]);
    g(&r.root, &r.env, &["commit", "-qm", "pkg"]);
    write(&r.root, "pkg/mod.py", "x = 2\n");
    let result = run(&r.root, &r.env, &["--thick"]);
    assert!(result.status.success(), "{}", stderr_of(&result));
    let out = text(&result.stdout);
    let tree = section(&result.stdout, "Macroscopic Map");
    assert!(tree.starts_with("```text\n."), "{tree}");
    assert!(tree.contains("mod.py"));
    let grounding = section(&result.stdout, "Contextual Grounding");
    assert_eq!(grounding, "# pkg/mod.py\n```py\nx = 2\n```\n</context>\n");
    assert!(out.find("### Diff").unwrap() < out.find("### Macroscopic Map").unwrap());
    assert!(stderr_of(&result).contains("info: Generating thick diff contextual grounding..."));
}

#[test]
fn outside_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let result = run(tmp.path(), &env, &[]);
    assert!(!result.status.success());
    assert_eq!(result.stdout, b"");
    assert!(stderr_of(&result).contains("not a git repository"));
}
