//! `issues` contracts against the fake GitHub API. Every run drives the picker through a
//! pseudo-terminal.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

const ESC: &[u8] = b"\x1b";
const ENTER: &[u8] = b"\r";
const CTRL_A: &[u8] = b"\x01";
const ISSUES: &str = "/repos/octo/widget/issues";

fn issue(number: u64, title: &str) -> Value {
    issue_with(number, title, json!(""), "open", false)
}

fn issue_with(number: u64, title: &str, body: Value, state: &str, pr: bool) -> Value {
    let mut item = json!({
        "number": number,
        "title": title,
        "body": body,
        "state": state,
        "user": {"login": "octocat"},
        "html_url": format!("https://github.com/octo/widget/issues/{number}"),
    });
    if pr {
        item["pull_request"] = json!({"url": "x"});
    }
    item
}

fn comment(author: &str, body: &str) -> Value {
    json!({"user": {"login": author}, "body": body})
}

fn detail(github: &FakeGitHub, number: u64, title: &str, body: &str) {
    github.add(
        &format!("{ISSUES}/{number}"),
        issue_with(number, title, json!(body), "open", false),
    );
    github.add(
        &format!("{ISSUES}/{number}/comments?per_page=100"),
        json!([]),
    );
}

fn headings(markdown: &str) -> Vec<&str> {
    markdown.lines().filter(|l| l.starts_with("# ")).collect()
}

fn next_link(github: &FakeGitHub, path: &str) -> String {
    format!("<{}>; rel=\"next\"", github.link(path))
}

/// Starts `focal <args>` on a pseudo-terminal with the fake's environment.
fn start(
    github: &FakeGitHub,
    tmp: &std::path::Path,
    cwd: &std::path::Path,
    args: &[&str],
) -> Session {
    let environment = github.env(tmp, &[]);
    let pairs: Vec<(&str, &str)> = environment
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    Session::start(cwd, args, &pairs)
}

/// Waits for the process to exit (without reaping it) and for the terminal output to
/// settle, then returns everything drawn.
fn screen_after_exit(session: &Session) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                session.pid() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc == 0 && unsafe { info.si_pid() } != 0 {
            break;
        }
        assert!(Instant::now() < deadline, "focal did not exit within 20 s");
        thread::sleep(Duration::from_millis(20));
    }
    let mut last = session.screen();
    loop {
        thread::sleep(Duration::from_millis(100));
        let now = session.screen();
        if now.len() == last.len() {
            return now;
        }
        last = now;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn lists_across_pages_without_pull_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add_with(
        "GET",
        &format!("{ISSUES}?state=open&per_page=100"),
        200,
        json!([
            issue(5, "Five"),
            issue_with(4, "Four PR", json!(""), "open", true),
            issue(3, "Three")
        ]),
        &[("Link", &next_link(&github, &format!("{ISSUES}?page=2")))],
    );
    github.add(&format!("{ISSUES}?page=2"), json!([issue(2, "Two")]));
    for (number, title) in [(5, "Five"), (3, "Three"), (2, "Two")] {
        detail(&github, number, title, &format!("{title} body"));
    }

    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("3/3");
    session.send(&[CTRL_A, ENTER]);
    let screen = screen_after_exit(&session);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(
        headings(&done.stdout),
        ["# Issue #5: Five", "# Issue #3: Three", "# Issue #2: Two"]
    );
    assert!(done.stdout.contains("\n\n---\n\n# Issue #3: Three"));
    assert!(contains(&screen, b"Three"));
    assert!(!contains(&screen, b"Four"));
    assert!(
        done.stderr.starts_with("info: Fetching issues...\n"),
        "{}",
        done.stderr
    );
    assert!(done.stderr.contains("done: Copied context for 3 issues (~"));
    let requests = github.requests();
    assert!(
        !requests
            .iter()
            .any(|r| r.path.starts_with(&format!("{ISSUES}/4")))
    );
    let auth: HashSet<_> = requests.iter().map(|r| r.header("Authorization")).collect();
    assert_eq!(auth, HashSet::from([Some("Bearer test-token")]));
}

#[test]
fn limits_the_list_to_100_issues() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let mut first: Vec<Value> = (231..=300)
        .rev()
        .map(|n| issue(n, &format!("Issue {n}")))
        .collect();
    first.extend(
        (221..=230)
            .rev()
            .map(|n| issue_with(n, &format!("PR {n}"), json!(""), "open", true)),
    );
    github.add_with(
        "GET",
        &format!("{ISSUES}?state=open&per_page=100"),
        200,
        Value::Array(first),
        &[("Link", &next_link(&github, &format!("{ISSUES}?page=2")))],
    );
    github.add_with(
        "GET",
        &format!("{ISSUES}?page=2"),
        200,
        Value::Array(
            (151..=220)
                .rev()
                .map(|n| issue(n, &format!("Issue {n}")))
                .collect(),
        ),
        &[("Link", &next_link(&github, &format!("{ISSUES}?page=3")))],
    );

    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("100/100");
    session.send(&[ESC]);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(done.stdout, "");
    assert!(!github.requests().iter().any(|r| r.path.contains("page=3")));
}

#[test]
fn all_includes_closed_issues() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{ISSUES}?state=all&per_page=100"),
        json!([
            issue(9, "Still open"),
            issue_with(8, "Fixed already", json!(""), "closed", false)
        ]),
    );
    detail(&github, 8, "Fixed already", "It was broken.");

    let session = start(&github, tmp.path(), &checkout, &["issues", "--all"]);
    session.wait_for("CLOSED");
    session.send(&[b"Fixed", ENTER]);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(headings(&done.stdout), ["# Issue #8: Fixed already"]);
    assert!(
        done.stderr.contains("done: Copied context for Issue #8 (~"),
        "{}",
        done.stderr
    );
}

#[test]
fn renders_the_full_comment_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{ISSUES}?state=open&per_page=100"),
        json!([issue(1, "Crash")]),
    );
    github.add(
        &format!("{ISSUES}/1"),
        issue_with(1, "Crash", Value::Null, "open", false),
    );
    github.add_with(
        "GET",
        &format!("{ISSUES}/1/comments?per_page=100"),
        200,
        json!([comment("ana", "First"), comment("bo", "Second")]),
        &[(
            "Link",
            &next_link(&github, &format!("{ISSUES}/1/comments?page=2")),
        )],
    );
    github.add(
        &format!("{ISSUES}/1/comments?page=2"),
        json!([comment("cy", "Third")]),
    );

    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("1/1");
    session.send(&[ENTER]);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(
        done.stdout,
        concat!(
            "<context>\n",
            "# Issue #1: Crash\n",
            "URL: https://github.com/octo/widget/issues/1\n\n",
            "## Description\n",
            "*No description provided.*\n\n",
            "## Discussion Thread\n\n",
            "### Comment 1 (@ana)\nFirst\n\n",
            "### Comment 2 (@bo)\nSecond\n\n",
            "### Comment 3 (@cy)\nThird\n",
            "</context>\n",
        )
    );
}

#[test]
fn one_failing_issue_does_not_abort_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{ISSUES}?state=open&per_page=100"),
        json!([issue(2, "Broken"), issue(1, "Fine")]),
    );
    github.add_with(
        "GET",
        &format!("{ISSUES}/2"),
        500,
        json!({"message": "Server Error"}),
        &[],
    );
    detail(&github, 1, "Fine", "ok");

    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("2/2");
    session.send(&[CTRL_A, ENTER]);
    let done = session.finish();

    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert!(
        done.stdout.contains(concat!(
            "# Issue #2\n\n[Could not fetch issue details or comments]\n\n---\n\n",
            "# Issue #1: Fine\n"
        )),
        "{}",
        done.stdout
    );
    assert!(
        done.stderr
            .contains("error: could not fetch issue #2: GitHub API error (HTTP 500)"),
        "{}",
        done.stderr
    );
    assert!(done.stderr.contains("done: Copied context for 2 issues"));
}

#[test]
fn previews_the_issue_body() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{ISSUES}?state=open&per_page=100"),
        json!([
            issue_with(1, "Leak", json!("Memory grows unbounded"), "open", false),
            issue(2, "Empty")
        ]),
    );
    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("unbounded");
    session.send(&[b"Empty"]);
    session.wait_for("description");
    session.send(&[ESC]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
}

#[test]
fn cancel_exits_quietly() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(
        &format!("{ISSUES}?state=open&per_page=100"),
        json!([issue(1, "One")]),
    );
    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    session.wait_for("Select issues: ");
    session.wait_for("1/1");
    session.send(&[b"\t", ESC]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(done.stdout, "");
    assert_eq!(done.stderr, "info: Fetching issues...\n");
}

#[test]
fn list_failure_exits_non_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add_with(
        "GET",
        &format!("{ISSUES}?state=open&per_page=100"),
        401,
        json!({"message": "Bad credentials"}),
        &[],
    );
    let session = start(&github, tmp.path(), &checkout, &["issues"]);
    let done = session.finish();
    assert_eq!(done.code, Some(1));
    assert_eq!(done.stdout, "");
    assert!(
        done.stderr
            .contains("error: GitHub rejected the credentials (HTTP 401): Bad credentials"),
        "{}",
        done.stderr
    );
}

#[test]
fn missing_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let environment: Env = github
        .env(tmp.path(), &[("PATH", "")])
        .into_iter()
        .filter(|(k, _)| k != "GH_TOKEN")
        .collect();
    let result = focal(&checkout, &environment, &["issues"], None);
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("error: no GitHub credentials found for github.com"),
        "{stderr}"
    );
    assert!(stderr.contains("set GH_TOKEN or GITHUB_TOKEN"), "{stderr}");
    assert!(github.requests().is_empty());
}

#[test]
fn missing_origin() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    git(&checkout, &["remote", "remove", "origin"]);
    let result = focal(&checkout, &github.env(tmp.path(), &[]), &["issues"], None);
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("error: this repository has no 'origin' remote"),
        "{stderr}"
    );
}
