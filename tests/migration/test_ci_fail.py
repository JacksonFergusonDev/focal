"""`ci-fail` contracts against the fake GitHub API.

The legacy command shells out to `gh run list` and `gh run view --log-failed`, so these
run only against the Rust binary. Expected log text comes from the legacy
`focal.gh_ci_fail.filter_logs` applied to the `job<TAB>step<TAB>line` text that
`gh run view --log-failed` prints, so the rendered output is checked against legacy
processing rather than a hand-written copy of it.
"""

import io
import os
import zipfile
from datetime import datetime, timedelta, timezone
from urllib.parse import parse_qs, urlsplit

import pytest

from focal.gh_ci_fail import filter_logs

pytestmark = pytest.mark.skipif(
    "FOCAL_TEST_BIN" not in os.environ, reason="legacy uses gh"
)

RUNS = "/repos/octo/widget/actions/runs"
JOBS = f"{RUNS}/101/jobs"
LOGS = f"{RUNS}/101/logs"
BLOB = "/blob/run-101.zip"
ESC, ENTER = b"\x1b", b"\r"


def make_zip(entries):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, text in entries.items():
            archive.writestr(name, text.encode())
    return buffer.getvalue()


def run(run_id=101, title="Fix the widget"):
    return {
        "id": run_id,
        "name": "CI",
        "display_title": title,
        "head_branch": "main",
        "head_sha": "0123456789abcdef",
        "event": "push",
        "created_at": "2026-11-24T10:00:00Z",
        "status": "completed",
        "conclusion": "failure",
        "html_url": f"https://github.com/octo/widget/actions/runs/{run_id}",
    }


def job(job_id, name, conclusion, steps=()):
    return {
        "id": job_id,
        "name": name,
        "conclusion": conclusion,
        "steps": [
            {"number": number, "name": step, "conclusion": result}
            for number, step, result in steps
        ],
    }


MIXED_JOBS = [
    job(1, "lint", "success", [(1, "Set up job", "success")]),
    job(
        2,
        "test",
        "failure",
        [
            (1, "Set up job", "success"),
            (2, "Checkout", "success"),
            (3, "Run tests", "failure"),
            (4, "Post Run", "success"),
        ],
    ),
]


def stamp(second):
    return f"2026-11-24T10:00:{second % 60:02d}.{second:07d}Z "


def failing_log():
    """A step log with ANSI color, a successful group, two separated errors, and a
    timestamp inside a message that must survive prefix stripping."""
    lines = [
        "##[group]Run pytest",
        "pytest -q",
        "##[endgroup]",
        "\x1b[1m===== test session starts =====\x1b[0m",
        *(f"collected item {i}" for i in range(20)),
        "\x1b[31mFAILED tests/test_widget.py::test_spin - AssertionError\x1b[0m",
        "at 2026-01-01T00:00:00Z the clock stopped",
        *(f"teardown {i}" for i in range(30)),
        "##[group]Upload report",
        "uploaded",
        "##[endgroup]",
        "##[error]Process completed with exit code 1.",
    ]
    return [stamp(i) + line for i, line in enumerate(lines)]


def gh_output(segments):
    """What `gh run view --log-failed` prints for (job, step, lines) segments."""
    return "".join(f"{j}\t{s}\t{line}\n" for j, s, lines in segments for line in lines)


def legacy_render(raw, title="CI - Fix the widget"):
    filtered = filter_logs(raw.strip()).strip()
    return (
        f"<context>\n# CI Failure Context: {title}\n\n"
        f"```text\n[error logs]\n{filtered}\n```\n</context>\n"
    )


def other_host(github, path):
    """The same fake server addressed by another host name, like GitHub's log storage."""
    return github.link(path).replace("127.0.0.1", "localhost")


def serve_run(github, jobs, entries):
    github.add(RUNS, {"total_count": 1, "workflow_runs": [run()]})
    github.add(JOBS, {"total_count": len(jobs), "jobs": jobs})
    github.add(LOGS, status=302, headers={"Location": other_host(github, BLOB)})
    github.add(BLOB, raw=make_zip(entries))


def select_first(terminal, cwd, github, tmp_path, *args):
    session = terminal(cwd, "ci-fail", *args, env=github.env(tmp_path))
    session.wait_for("Select failed CI run: ")
    session.wait_for("101  [completed/failure] 2026-11-24T10:00:00Z | CI (main)")
    return session


def header(request, name):
    return {key.lower(): value for key, value in request.headers.items()}.get(name)


def paths(github, path):
    return [r for r in github.requests if urlsplit(r.path).path == path]


def test_default_lists_failed_runs_from_the_last_day(
    github, github_checkout, focal, tmp_path
):
    github.add(RUNS, {"total_count": 0, "workflow_runs": []})
    before = datetime.now(timezone.utc) - timedelta(days=1)
    result = focal(github_checkout, github.env(tmp_path), "ci-fail")
    after = datetime.now(timezone.utc) - timedelta(days=1)
    assert result.returncode == 0, result.stderr
    assert result.stdout == b""
    assert result.stderr.decode() == (
        "info: No failed CI runs found in the last 24 hours. "
        "Run with -a or --all to view historical runs.\n"
    )
    (request,) = paths(github, RUNS)
    query = parse_qs(urlsplit(request.path).query)
    assert query["per_page"] == ["30"]
    assert query["status"] == ["failure"]
    (created,) = query["created"]
    assert created.startswith(">=")
    cutoff = datetime.strptime(created[2:], "%Y-%m-%dT%H:%M:%SZ").replace(
        tzinfo=timezone.utc
    )
    assert before - timedelta(seconds=1) <= cutoff <= after
    assert header(request, "authorization") == "Bearer test-token"


@pytest.mark.parametrize("flag", ["-a", "--all"])
def test_all_drops_the_time_filter_but_keeps_the_limit(
    github, github_checkout, focal, tmp_path, flag
):
    github.add(RUNS, {"total_count": 0, "workflow_runs": []})
    result = focal(github_checkout, github.env(tmp_path), "ci-fail", flag)
    assert result.returncode == 0, result.stderr
    assert result.stderr.decode() == "info: No failed CI runs found.\n"
    (request,) = paths(github, RUNS)
    query = parse_qs(urlsplit(request.path).query)
    assert query == {"per_page": ["30"], "status": ["failure"]}


def test_missing_credentials(github, github_checkout, focal, tmp_path):
    env = github.env(tmp_path, PATH="")
    del env["GH_TOKEN"]
    result = focal(github_checkout, env, "ci-fail")
    assert result.returncode == 1
    assert "error: no GitHub credentials found for github.com" in result.stderr.decode()
    assert "hint: set GH_TOKEN or GITHUB_TOKEN" in result.stderr.decode()
    assert github.requests == []


def test_picker_cancel_exits_quietly(github, github_checkout, terminal, tmp_path):
    serve_run(github, MIXED_JOBS, {})
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.send(ESC)
    code, err, out, _ = session.finish()
    assert (code, err, out) == (0, "", "")
    assert paths(github, LOGS) == []


def test_mixed_success_run_renders_only_failed_steps(
    github, github_checkout, terminal, tmp_path
):
    log = failing_log()
    noise = "2026-11-24T10:00:00.0000000Z SHOULD NOT APPEAR error: nope"
    serve_run(
        github,
        MIXED_JOBS,
        {
            "0_lint.txt": noise,
            "1_test.txt": noise,
            "lint/1_Set up job.txt": noise,
            "test/1_Set up job.txt": noise,
            "test/2_Checkout.txt": noise,
            "test/3_Run tests.txt": "﻿" + "\r\n".join(log) + "\r\n",
            "test/4_Post Run.txt": noise,
        },
    )
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.wait_for("\u2717")  # the preview marks the failed step
    session.send(ENTER)
    code, err, out, _ = session.finish()
    assert code == 0, err
    assert out == legacy_render(gh_output([("test", "Run tests", log)]))
    assert "SHOULD NOT APPEAR" not in out
    assert err.startswith("done: Copied CI failure logs for run 101 (~")

    # ANSI codes and leading timestamps are gone; message timestamps are kept.
    assert "\x1b" not in out
    assert "2026-11-24T10:00" not in out
    assert "\nat 2026-01-01T00:00:00Z the clock stopped\n" in out
    assert "\nFAILED tests/test_widget.py::test_spin - AssertionError\n" in out
    # Two error windows joined by a gap marker, and the successful group collapsed.
    assert "\nteardown 8\n...\nteardown 16\n" in out
    assert "> [Group completed successfully: Upload report]\n##[error]Process" in out
    assert "collected item 3\n" not in out


def test_log_download_drops_authorization_on_the_cross_host_redirect(
    github, github_checkout, terminal, tmp_path
):
    serve_run(github, MIXED_JOBS, {"test/3_Run tests.txt": "\n".join(failing_log())})
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.send(ENTER)
    code, err, _, _ = session.finish()
    assert code == 0, err
    (api,) = paths(github, LOGS)
    (blob,) = paths(github, BLOB)
    assert header(api, "authorization") == "Bearer test-token"
    assert header(blob, "host").startswith("localhost:")
    assert header(blob, "authorization") is None


def test_jobs_without_step_logs_use_the_job_log(
    github, github_checkout, terminal, tmp_path
):
    jobs = [
        job(7, "build", "failure", [(1, "Compile", "failure")]),
        job(8, "docs", "timed_out", [(1, "Render", "failure")]),
        job(9, "deploy", "cancelled", [(1, "Ship", "cancelled")]),
    ]
    build = [stamp(1) + "compiling", stamp(2) + "error: missing semicolon"]
    docs = [stamp(3) + "rendering", stamp(4) + "Error: timed out"]
    serve_run(
        github, jobs, {"0_build.txt": "\n".join(build), "2_deploy.txt": "SHIPPED"}
    )
    github.add(
        "/repos/octo/widget/actions/jobs/8/logs",
        status=302,
        headers={"Location": other_host(github, "/blob/job-8.txt")},
    )
    github.add("/blob/job-8.txt", raw="\n".join(docs).encode())
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.send(ENTER)
    code, err, out, _ = session.finish()
    assert code == 0, err
    raw = gh_output([("build", "UNKNOWN STEP", build), ("docs", "UNKNOWN STEP", docs)])
    assert out == legacy_render(raw)
    assert "SHIPPED" not in out
    (job_log,) = paths(github, "/blob/job-8.txt")
    assert header(job_log, "authorization") is None


@pytest.mark.parametrize("status", [404, 410])
def test_expired_logs(github, github_checkout, terminal, tmp_path, status):
    github.add(RUNS, {"total_count": 1, "workflow_runs": [run()]})
    github.add(JOBS, {"total_count": 2, "jobs": MIXED_JOBS})
    github.add(LOGS, {"message": "Gone"}, status=status)
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.send(ENTER)
    code, err, out, _ = session.finish()
    assert code == 1
    assert out == ""
    assert (
        f"error: the logs for run 101 have expired or were deleted (HTTP {status})"
        in err
    )
    assert "re-run the workflow" in err


def test_run_without_failed_steps(github, github_checkout, terminal, tmp_path):
    jobs = [job(1, "lint", "cancelled", [(1, "Set up job", "cancelled")])]
    serve_run(github, jobs, {})
    session = select_first(terminal, github_checkout, github, tmp_path)
    session.wait_for("jobs.")  # the preview reports "No failed jobs."
    session.send(ENTER)
    code, err, out, _ = session.finish()
    assert code == 1
    assert out == ""
    assert "error: run 101 has no failed steps with logs" in err
    assert "https://github.com/octo/widget/actions/runs/101" in err
    assert paths(github, LOGS) == []
