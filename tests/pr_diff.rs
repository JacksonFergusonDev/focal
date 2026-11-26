//! `pr-diff` contracts against the fake GitHub API.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const ESC: &[u8] = b"\x1b";
const ENTER: &[u8] = b"\r";
const PULLS: &str = "/repos/octo/widget/pulls";
const PATCH: &str = concat!(
    "diff --git a/app.py b/app.py\n",
    "--- a/app.py\n+++ b/app.py\n",
    "@@ -1 +1 @@\n-old\n+new\n",
);
const WHY: &str = "Why this matters.";

fn pull(number: u64, title: &str) -> Value {
    pull_with(number, title, json!(WHY), "feature")
}

fn pull_with(number: u64, title: &str, body: Value, head: &str) -> Value {
    json!({
        "number": number,
        "title": title,
        "body": body,
        "state": "open",
        "head": {"ref": head},
        "html_url": format!("https://github.com/octo/widget/pull/{number}"),
    })
}

/// The PR JSON then its patch: both live at the same path.
fn serve(github: &FakeGitHub, pull: Value) {
    let path = format!("{PULLS}/{}", pull["number"]);
    github.add(&path, pull);
    github.add_raw(&path, PATCH);
}

fn serve_pull(github: &FakeGitHub, number: u64, title: &str) {
    serve(github, pull(number, title));
}

fn expected(number: u64, title: &str, body: &str) -> String {
    format!(
        "<context>\n\
         # PR #{number}: {title}\n\
         URL: https://github.com/octo/widget/pull/{number}\n\n\
         ## Intent / Description\n\
         {body}\n\n\
         ## Diff\n\
         ```diff\n{}\n```\n\
         </context>\n",
        PATCH.trim()
    )
}

fn pr_env(github: &FakeGitHub, tmp: &Path) -> Env {
    github.env(tmp, &[("FOCAL_OMIT_WORKSPACE", "1")])
}

fn start(environment: &Env, cwd: &Path, args: &[&str]) -> Session {
    let pairs: Vec<(&str, &str)> = environment
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    Session::start(cwd, args, &pairs)
}

/// Runs focal in a new session with no controlling terminal and stdin from /dev/null.
fn focal_without_terminal(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(cwd)
        .clean_env()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().expect("run focal")
}

#[test]
fn explicit_number() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    serve_pull(&github, 7, "Add widget");
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "7"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(text(&result.stdout), expected(7, "Add widget", WHY));
    assert!(text(&result.stderr).starts_with("done: Copied context and diff for PR #7"));
    let requests = github.requests();
    let accepts: Vec<_> = requests.iter().map(|r| r.header("Accept")).collect();
    assert_eq!(
        accepts,
        [
            Some("application/vnd.github+json"),
            Some("application/vnd.github.diff")
        ]
    );
    let auth: HashSet<_> = requests.iter().map(|r| r.header("Authorization")).collect();
    assert_eq!(auth, HashSet::from([Some("Bearer test-token")]));
}

#[test]
fn explicit_url_and_empty_body() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    serve(&github, pull_with(12, "Docs", Value::Null, "feature"));
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "https://github.com/octo/widget/pull/12/files"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(
        text(&result.stdout),
        expected(12, "Docs", "*No description provided.*")
    );
}

fn url_for_another_repository(outside: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let other = "/repos/other/lib/pulls/42";
    let mut body = pull(42, "Upstream fix");
    body["html_url"] = json!("https://github.com/other/lib/pull/42");
    github.add(other, body);
    github.add_raw(other, PATCH);
    let cwd = if outside { tmp.path() } else { &checkout };
    let result = focal(
        cwd,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "https://github.com/other/lib/pull/42/files"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).starts_with("<context>\n# PR #42: Upstream fix\n"));
    let paths: Vec<_> = github.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, [other, other]);
}

#[test]
fn url_for_another_repository_from_unrelated_checkout() {
    url_for_another_repository(false);
}

#[test]
fn url_for_another_repository_from_outside_any_repository() {
    url_for_another_repository(true);
}

#[test]
fn branch_name_containing_pull_is_not_a_url() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{PULLS}?head=octo:release/pull/3&state=all&per_page=100"),
        json!([pull(9, "Release")]),
    );
    serve_pull(&github, 9, "Release");
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "release/pull/3"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("# PR #9: Release"));
}

#[test]
fn explicit_branch_name() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{PULLS}?head=octo:feature/login&state=all&per_page=100"),
        json!([pull(3, "Login")]),
    );
    serve_pull(&github, 3, "Login");
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "feature/login"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("# PR #3: Login"));
}

#[test]
fn current_branch_pull_request() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let mut closed = pull(4, "Old attempt");
    closed["state"] = json!("closed");
    github.add(
        &format!("{PULLS}?head=octo:main&state=all&per_page=100"),
        json!([closed, pull(5, "Current")]),
    );
    serve_pull(&github, 5, "Current");
    let result = focal(&checkout, &pr_env(&github, tmp.path()), &["pr-diff"], None);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(text(&result.stdout), expected(5, "Current", WHY));
}

#[test]
fn current_branch_uses_upstream_name() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    git(
        &checkout,
        &["config", "branch.main.merge", "refs/heads/topic"],
    );
    github.add(
        &format!("{PULLS}?head=octo:topic&state=all&per_page=100"),
        json!([pull(6, "Topic")]),
    );
    serve_pull(&github, 6, "Topic");
    let result = focal(&checkout, &pr_env(&github, tmp.path()), &["pr-diff"], None);
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("# PR #6: Topic"));
}

fn lookup_falls_back_to_the_picker(lookup_fails: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let query = format!("{PULLS}?head=octo:main&state=all&per_page=100");
    if lookup_fails {
        github.add_with("GET", &query, 500, json!({"message": "boom"}), &[]);
    } else {
        github.add(&query, json!([]));
    }
    github.add(
        &format!("{PULLS}?state=open&per_page=30"),
        json!([
            pull_with(21, "Refactor", json!(WHY), "cleanup"),
            pull_with(20, "Add cache", json!(WHY), "cache")
        ]),
    );
    serve_pull(&github, 20, "Add cache");

    let session = start(&pr_env(&github, tmp.path()), &checkout, &["pr-diff"]);
    session.wait_for("Select PR: ");
    session.wait_for("[cleanup] Refactor");
    session.send(&[b"cache", ENTER]);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(done.stdout, expected(20, "Add cache", WHY));
    assert!(!done.stderr.contains("boom"), "{}", done.stderr);
}

#[test]
fn lookup_falls_back_to_the_picker_when_none() {
    lookup_falls_back_to_the_picker(false);
}

#[test]
fn lookup_falls_back_to_the_picker_on_error() {
    lookup_falls_back_to_the_picker(true);
}

#[test]
fn picker_cancel_exits_quietly() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{PULLS}?head=octo:main&state=all&per_page=100"),
        json!([]),
    );
    github.add(
        &format!("{PULLS}?state=open&per_page=30"),
        json!([pull(1, "Only")]),
    );
    let session = start(&pr_env(&github, tmp.path()), &checkout, &["pr-diff"]);
    session.wait_for("[feature] Only");
    session.send(&[ESC]);
    let done = session.finish();
    assert_eq!(
        (done.code, done.stderr.as_str(), done.stdout.as_str()),
        (Some(0), "", "")
    );
}

#[test]
fn no_terminal_for_the_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{PULLS}?head=octo:main&state=all&per_page=100"),
        json!([]),
    );
    let result = focal_without_terminal(&checkout, &pr_env(&github, tmp.path()), &["pr-diff"]);
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("no pull request found for the current branch"),
        "{stderr}"
    );
    assert!(stderr.contains("focal pr-diff <number>"), "{stderr}");
}

#[test]
fn missing_pull_request() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add_with(
        "GET",
        &format!("{PULLS}/404"),
        404,
        json!({"message": "Not Found"}),
        &[],
    );
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "404"],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("error: not found on GitHub (HTTP 404)"));
    assert!(result.stdout.is_empty());
}

#[test]
fn patch_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(&format!("{PULLS}/8"), pull(8, "Huge"));
    github.add_with(
        "GET",
        &format!("{PULLS}/8"),
        406,
        json!({"message": "diff too large"}),
        &[],
    );
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "8"],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("GitHub API error (HTTP 406): diff too large"),
        "{stderr}"
    );
    assert!(
        stderr.contains("verify that the PR exists and you have access"),
        "{stderr}"
    );
}

#[test]
fn missing_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let environment: Env = with(&pr_env(&github, tmp.path()), &[("PATH", "")])
        .into_iter()
        .filter(|(k, _)| k != "GH_TOKEN")
        .collect();
    let result = focal(&checkout, &environment, &["pr-diff", "7"], None);
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("error: no GitHub credentials found for github.com"));
    assert!(github.requests().is_empty());
}

#[test]
fn diff_that_is_not_utf8_is_decoded_lossily() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(&format!("{PULLS}/3"), pull(3, "Latin-1"));
    github.add_raw(
        &format!("{PULLS}/3"),
        &b"diff --git a/l b/l\n+caf\xe9\n"[..],
    );
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "3"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    // Decoded strictly, as the Python test did, so stray invalid bytes would fail here.
    let stdout = String::from_utf8(result.stdout).expect("stdout is UTF-8");
    assert!(stdout.contains("+caf\u{FFFD}"), "{stdout}");
}

#[test]
fn too_many_requests_is_a_rate_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add_with(
        "GET",
        &format!("{PULLS}/4"),
        429,
        json!({"message": "slow down"}),
        &[("Retry-After", "60")],
    );
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "4"],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(stderr.contains("rate limit"), "{stderr}");
    assert!(stderr.contains("wait 60 seconds"), "{stderr}");
}

#[test]
fn credentials_in_origin_are_not_printed() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    git(
        &checkout,
        &[
            "remote",
            "set-url",
            "origin",
            "https://x-access-token:ghp_SECRET@github.com/",
        ],
    );
    let result = focal(
        &checkout,
        &pr_env(&github, tmp.path()),
        &["pr-diff", "1"],
        None,
    );
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("could not read a GitHub owner and repository"),
        "{stderr}"
    );
    assert!(!stderr.contains("ghp_SECRET"));
    assert!(github.requests().is_empty());
}
