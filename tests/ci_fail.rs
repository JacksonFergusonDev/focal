//! `ci-fail` contracts against the fake GitHub API.
//!
//! The expected log text in `tests/fixtures/ci_fail/` was recorded from the legacy Python
//! filter (`focal.gh_ci_fail.filter_logs`) applied to the `job<TAB>step<TAB>line` text that
//! `gh run view --log-failed` printed for the same step logs these tests serve, so the
//! rendered output is checked against legacy processing rather than a hand-written copy.

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const RUNS: &str = "/repos/octo/widget/actions/runs";
const JOBS: &str = "/repos/octo/widget/actions/runs/101/jobs";
const LOGS: &str = "/repos/octo/widget/actions/runs/101/logs";
const BLOB: &str = "/blob/run-101.zip";
const ESC: &[u8] = b"\x1b";
const ENTER: &[u8] = b"\r";

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ci_fail")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in entries {
        archive.start_file(*name, options).unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

fn run() -> Value {
    json!({
        "id": 101,
        "name": "CI",
        "display_title": "Fix the widget",
        "head_branch": "main",
        "head_sha": "0123456789abcdef",
        "event": "push",
        "created_at": "2026-11-24T10:00:00Z",
        "status": "completed",
        "conclusion": "failure",
        "html_url": "https://github.com/octo/widget/actions/runs/101",
    })
}

fn job(id: u64, name: &str, conclusion: &str, steps: &[(u64, &str, &str)]) -> Value {
    let steps: Vec<Value> = steps
        .iter()
        .map(|(number, step, result)| json!({"number": number, "name": step, "conclusion": result}))
        .collect();
    json!({"id": id, "name": name, "conclusion": conclusion, "steps": steps})
}

fn mixed_jobs() -> Vec<Value> {
    vec![
        job(1, "lint", "success", &[(1, "Set up job", "success")]),
        job(
            2,
            "test",
            "failure",
            &[
                (1, "Set up job", "success"),
                (2, "Checkout", "success"),
                (3, "Run tests", "failure"),
                (4, "Post Run", "success"),
            ],
        ),
    ]
}

fn stamp(second: usize) -> String {
    format!("2026-11-24T10:00:{:02}.{second:07}Z ", second % 60)
}

/// A step log with ANSI color, a successful group, two separated errors, and a timestamp
/// inside a message that must survive prefix stripping.
fn failing_log() -> Vec<String> {
    let mut lines: Vec<String> = vec![
        "##[group]Run pytest".into(),
        "pytest -q".into(),
        "##[endgroup]".into(),
        "\x1b[1m===== test session starts =====\x1b[0m".into(),
    ];
    lines.extend((0..20).map(|i| format!("collected item {i}")));
    lines.push("\x1b[31mFAILED tests/test_widget.py::test_spin - AssertionError\x1b[0m".into());
    lines.push("at 2026-01-01T00:00:00Z the clock stopped".into());
    lines.extend((0..30).map(|i| format!("teardown {i}")));
    lines.extend([
        "##[group]Upload report".into(),
        "uploaded".into(),
        "##[endgroup]".into(),
        "##[error]Process completed with exit code 1.".into(),
    ]);
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| stamp(i) + &line)
        .collect()
}

/// The same fake server addressed by another host name, like GitHub's log storage.
fn other_host(github: &FakeGitHub, path: &str) -> String {
    github.link(path).replace("127.0.0.1", "localhost")
}

fn serve_run(github: &FakeGitHub, jobs: Vec<Value>, entries: &[(&str, &str)]) {
    github.add(RUNS, json!({"total_count": 1, "workflow_runs": [run()]}));
    github.add(JOBS, json!({"total_count": jobs.len(), "jobs": jobs}));
    github.add_empty("GET", LOGS, 302, &[("Location", &other_host(github, BLOB))]);
    github.add_raw(BLOB, make_zip(entries));
}

fn select_first(github: &FakeGitHub, tmp: &Path, cwd: &Path, extra_args: &[&str]) -> Session {
    let environment = github.env(tmp, &[]);
    let pairs: Vec<(&str, &str)> = environment
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut args = vec!["ci-fail"];
    args.extend_from_slice(extra_args);
    let session = Session::start(cwd, &args, &pairs);
    session.wait_for("Select failed CI run: ");
    session.wait_for("101  [completed/failure] 2026-11-24T10:00:00Z | CI (main)");
    session
}

/// Requests whose path, without the query, is `path`.
fn paths(github: &FakeGitHub, path: &str) -> Vec<Request> {
    github
        .requests()
        .into_iter()
        .filter(|r| r.path.split('?').next() == Some(path))
        .collect()
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// Like Python's `parse_qs`: each key's values, in order.
fn query(request: &Request) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let Some((_, query)) = request.path.split_once('?') else {
        return out;
    };
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.entry(percent_decode(key))
            .or_default()
            .push(percent_decode(value));
    }
    out
}

/// Seconds since the epoch for a `%Y-%m-%dT%H:%M:%SZ` timestamp.
fn parse_utc(stamp: &str) -> i64 {
    let n = |range: std::ops::Range<usize>| stamp[range].parse::<i64>().unwrap();
    assert_eq!(stamp.len(), 20, "{stamp}");
    assert!(stamp.ends_with('Z') && &stamp[10..11] == "T", "{stamp}");
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let (hh, mm, ss) = (n(11..13), n(14..16), n(17..19));
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hh * 3600 + mm * 60 + ss
}

fn day_ago() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        - 86_400.0
}

#[test]
fn default_lists_failed_runs_from_the_last_day() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(RUNS, json!({"total_count": 0, "workflow_runs": []}));
    let before = day_ago();
    let result = focal(&checkout, &github.env(tmp.path(), &[]), &["ci-fail"], None);
    let after = day_ago();
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert!(result.stdout.is_empty());
    assert_eq!(
        text(&result.stderr),
        "info: No failed CI runs found in the last 24 hours. \
         Run with -a or --all to view historical runs.\n"
    );
    let requests = paths(&github, RUNS);
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let query = query(request);
    assert_eq!(query["per_page"], ["30"]);
    assert_eq!(query["status"], ["failure"]);
    assert_eq!(query["created"].len(), 1);
    let created = &query["created"][0];
    let cutoff = created
        .strip_prefix(">=")
        .unwrap_or_else(|| panic!("created={created}"));
    let cutoff = parse_utc(cutoff) as f64;
    assert!(
        before - 1.0 <= cutoff && cutoff <= after,
        "{before} <= {cutoff} <= {after}"
    );
    assert_eq!(request.header("authorization"), Some("Bearer test-token"));
}

fn all_drops_the_time_filter_but_keeps_the_limit(flag: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(RUNS, json!({"total_count": 0, "workflow_runs": []}));
    let result = focal(
        &checkout,
        &github.env(tmp.path(), &[]),
        &["ci-fail", flag],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(text(&result.stderr), "info: No failed CI runs found.\n");
    let requests = paths(&github, RUNS);
    assert_eq!(requests.len(), 1);
    let expected: HashMap<String, Vec<String>> = HashMap::from([
        ("per_page".to_owned(), vec!["30".to_owned()]),
        ("status".to_owned(), vec!["failure".to_owned()]),
    ]);
    assert_eq!(query(&requests[0]), expected);
}

#[test]
fn all_drops_the_time_filter_but_keeps_the_limit_short_flag() {
    all_drops_the_time_filter_but_keeps_the_limit("-a");
}

#[test]
fn all_drops_the_time_filter_but_keeps_the_limit_long_flag() {
    all_drops_the_time_filter_but_keeps_the_limit("--all");
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
    let result = focal(&checkout, &environment, &["ci-fail"], None);
    assert_eq!(result.status.code(), Some(1));
    let stderr = text(&result.stderr);
    assert!(
        stderr.contains("error: no GitHub credentials found for github.com"),
        "{stderr}"
    );
    assert!(
        stderr.contains("hint: set GH_TOKEN or GITHUB_TOKEN"),
        "{stderr}"
    );
    assert!(github.requests().is_empty());
}

#[test]
fn picker_cancel_exits_quietly() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    serve_run(&github, mixed_jobs(), &[]);
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.send(&[ESC]);
    let done = session.finish();
    assert_eq!(
        (done.code, done.stderr.as_str(), done.stdout.as_str()),
        (Some(0), "", "")
    );
    assert!(paths(&github, LOGS).is_empty());
}

#[test]
fn mixed_success_run_renders_only_failed_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let log = failing_log();
    let noise = "2026-11-24T10:00:00.0000000Z SHOULD NOT APPEAR error: nope";
    let failing = format!("\u{FEFF}{}\r\n", log.join("\r\n"));
    serve_run(
        &github,
        mixed_jobs(),
        &[
            ("0_lint.txt", noise),
            ("1_test.txt", noise),
            ("lint/1_Set up job.txt", noise),
            ("test/1_Set up job.txt", noise),
            ("test/2_Checkout.txt", noise),
            ("test/3_Run tests.txt", &failing),
            ("test/4_Post Run.txt", noise),
        ],
    );
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.wait_for("\u{2717}"); // the preview marks the failed step
    session.send(&[ENTER]);
    let done = session.finish();
    let out = &done.stdout;
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(*out, fixture("mixed_success_run.txt"));
    assert!(!out.contains("SHOULD NOT APPEAR"));
    assert!(
        done.stderr
            .starts_with("done: Copied CI failure logs for run 101 (~"),
        "{}",
        done.stderr
    );

    // ANSI codes and leading timestamps are gone; message timestamps are kept.
    assert!(!out.contains('\x1b'));
    assert!(!out.contains("2026-11-24T10:00"));
    assert!(out.contains("\nat 2026-01-01T00:00:00Z the clock stopped\n"));
    assert!(out.contains("\nFAILED tests/test_widget.py::test_spin - AssertionError\n"));
    // Two error windows joined by a gap marker, and the successful group collapsed.
    assert!(out.contains("\nteardown 8\n...\nteardown 16\n"));
    assert!(out.contains("> [Group completed successfully: Upload report]\n##[error]Process"));
    assert!(!out.contains("collected item 3\n"));
}

#[test]
fn log_download_drops_authorization_on_the_cross_host_redirect() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let log = failing_log().join("\n");
    serve_run(&github, mixed_jobs(), &[("test/3_Run tests.txt", &log)]);
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.send(&[ENTER]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    let api = paths(&github, LOGS);
    let blob = paths(&github, BLOB);
    assert_eq!((api.len(), blob.len()), (1, 1));
    assert_eq!(api[0].header("authorization"), Some("Bearer test-token"));
    assert!(blob[0].header("host").unwrap().starts_with("localhost:"));
    assert_eq!(blob[0].header("authorization"), None);
}

#[test]
fn jobs_without_step_logs_use_the_job_log() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let jobs = vec![
        job(7, "build", "failure", &[(1, "Compile", "failure")]),
        job(8, "docs", "timed_out", &[(1, "Render", "failure")]),
        job(9, "deploy", "cancelled", &[(1, "Ship", "cancelled")]),
    ];
    let build = [
        stamp(1) + "compiling",
        stamp(2) + "error: missing semicolon",
    ]
    .join("\n");
    let docs = [stamp(3) + "rendering", stamp(4) + "Error: timed out"].join("\n");
    serve_run(
        &github,
        jobs,
        &[("0_build.txt", &build), ("2_deploy.txt", "SHIPPED")],
    );
    github.add_empty(
        "GET",
        "/repos/octo/widget/actions/jobs/8/logs",
        302,
        &[("Location", &other_host(&github, "/blob/job-8.txt"))],
    );
    github.add_raw("/blob/job-8.txt", docs);
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.send(&[ENTER]);
    let done = session.finish();
    assert_eq!(done.code, Some(0), "{}", done.stderr);
    assert_eq!(done.stdout, fixture("jobs_without_step_logs.txt"));
    assert!(!done.stdout.contains("SHIPPED"));
    let job_log = paths(&github, "/blob/job-8.txt");
    assert_eq!(job_log.len(), 1);
    assert_eq!(job_log[0].header("authorization"), None);
}

fn expired_logs(status: u16) {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    github.add(RUNS, json!({"total_count": 1, "workflow_runs": [run()]}));
    github.add(JOBS, json!({"total_count": 2, "jobs": mixed_jobs()}));
    github.add_with("GET", LOGS, status, json!({"message": "Gone"}), &[]);
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.send(&[ENTER]);
    let done = session.finish();
    assert_eq!(done.code, Some(1), "{}", done.stderr);
    assert_eq!(done.stdout, "");
    assert!(
        done.stderr.contains(&format!(
            "error: the logs for run 101 have expired or were deleted (HTTP {status})"
        )),
        "{}",
        done.stderr
    );
    assert!(
        done.stderr.contains("re-run the workflow"),
        "{}",
        done.stderr
    );
}

#[test]
fn expired_logs_404() {
    expired_logs(404);
}

#[test]
fn expired_logs_410() {
    expired_logs(410);
}

#[test]
fn run_without_failed_steps() {
    let tmp = tempfile::tempdir().unwrap();
    let github = FakeGitHub::start();
    let checkout = github_checkout(tmp.path());
    let jobs = vec![job(
        1,
        "lint",
        "cancelled",
        &[(1, "Set up job", "cancelled")],
    )];
    serve_run(&github, jobs, &[]);
    let session = select_first(&github, tmp.path(), &checkout, &[]);
    session.wait_for("jobs."); // the preview reports "No failed jobs."
    session.send(&[ENTER]);
    let done = session.finish();
    assert_eq!(done.code, Some(1), "{}", done.stderr);
    assert_eq!(done.stdout, "");
    assert!(
        done.stderr
            .contains("error: run 101 has no failed steps with logs"),
        "{}",
        done.stderr
    );
    assert!(
        done.stderr
            .contains("https://github.com/octo/widget/actions/runs/101")
    );
    assert!(paths(&github, LOGS).is_empty());
}
