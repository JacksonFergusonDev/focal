//! Self-checks for the shared test harness.

mod common;

use common::*;
use serde_json::json;

#[test]
fn fake_github_serves_replies_in_order_and_records_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    github.add("/repos/octo/widget/pulls/1", json!({"number": 1, "title": "One", "body": "", "html_url": "u", "head": {"ref": "x"}, "state": "open"}));
    github.add_raw("/repos/octo/widget/pulls/1", "diff --git a/x b/x\n+y\n");
    let checkout = github_checkout(tmp.path());
    let out = focal(
        &checkout,
        &github.env(tmp.path(), &[("FOCAL_OMIT_WORKSPACE", "1")]),
        &["pr-diff", "1"],
        None,
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("# PR #1: One"));
    let requests = github.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].header("Authorization"),
        Some("Bearer test-token")
    );
}

#[test]
fn session_drives_a_picker_and_restores_the_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("project");
    write(&cwd, "alpha.txt", "alpha\n");
    let session = Session::start(&cwd, &["files"], &[]);
    session.wait_for("alpha.txt");
    session.send(&[b"\t", b"\r"]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert!(done.stdout.contains("alpha"), "{}", done.stdout);
    assert!(same_termios(&done.before, &done.after));
}
