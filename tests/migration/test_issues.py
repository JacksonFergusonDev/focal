"""`issues` contracts for the Rust CLI against the fake GitHub API.

The legacy command shells out to `gh` and fzf, so these run only against the Rust binary.
Every run drives the picker through a pseudo-terminal.
"""

import os
import subprocess

import pytest

pytestmark = pytest.mark.skipif(
    "FOCAL_TEST_BIN" not in os.environ, reason="legacy uses gh"
)

ESC, ENTER, CTRL_A = b"\x1b", b"\r", b"\x01"
ISSUES = "/repos/octo/widget/issues"


def issue(number, title, body="", state="open", author="octocat", pr=False):
    item = {
        "number": number,
        "title": title,
        "body": body,
        "state": state,
        "user": {"login": author},
        "html_url": f"https://github.com/octo/widget/issues/{number}",
    }
    if pr:
        item["pull_request"] = {"url": "x"}
    return item


def comment(author, body):
    return {"user": {"login": author}, "body": body}


def detail(github, number, title, body="", comments=()):
    github.add(f"{ISSUES}/{number}", issue(number, title, body))
    github.add(f"{ISSUES}/{number}/comments?per_page=100", list(comments))


def header(request, name):
    return {k.lower(): v for k, v in request.headers.items()}[name.lower()]


def headings(markdown):
    return [line for line in markdown.splitlines() if line.startswith("# ")]


def test_lists_across_pages_without_pull_requests(
    github, github_checkout, tmp_path, terminal
):
    github.add(
        f"{ISSUES}?state=open&per_page=100",
        [issue(5, "Five"), issue(4, "Four PR", pr=True), issue(3, "Three")],
        headers={"Link": f'<{github.link(ISSUES + "?page=2")}>; rel="next"'},
    )
    github.add(f"{ISSUES}?page=2", [issue(2, "Two")])
    for number, title in [(5, "Five"), (3, "Three"), (2, "Two")]:
        detail(github, number, title, body=f"{title} body")

    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("3/3")
    session.send(CTRL_A, ENTER)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert headings(out) == [
        "# Issue #5: Five",
        "# Issue #3: Three",
        "# Issue #2: Two",
    ]
    assert "\n\n---\n\n# Issue #3: Three" in out
    assert b"Three" in session.screen
    assert b"Four" not in session.screen
    assert err.startswith("info: Fetching issues...\n")
    assert "done: Copied context for 3 issues (~" in err
    assert not any(r.path.startswith(f"{ISSUES}/4") for r in github.requests)
    assert {header(r, "Authorization") for r in github.requests} == {
        "Bearer test-token"
    }


def test_limits_the_list_to_100_issues(github, github_checkout, tmp_path, terminal):
    first = [issue(n, f"Issue {n}") for n in range(300, 230, -1)]
    first += [issue(n, f"PR {n}", pr=True) for n in range(230, 220, -1)]
    github.add(
        f"{ISSUES}?state=open&per_page=100",
        first,
        headers={"Link": f'<{github.link(ISSUES + "?page=2")}>; rel="next"'},
    )
    github.add(
        f"{ISSUES}?page=2",
        [issue(n, f"Issue {n}") for n in range(220, 150, -1)],
        headers={"Link": f'<{github.link(ISSUES + "?page=3")}>; rel="next"'},
    )

    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("100/100")
    session.send(ESC)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert out == ""
    assert not any("page=3" in r.path for r in github.requests)


def test_all_includes_closed_issues(github, github_checkout, tmp_path, terminal):
    github.add(
        f"{ISSUES}?state=all&per_page=100",
        [issue(9, "Still open"), issue(8, "Fixed already", state="closed")],
    )
    detail(github, 8, "Fixed already", body="It was broken.")

    session = terminal(github_checkout, "issues", "--all", env=github.env(tmp_path))
    session.wait_for("CLOSED")
    session.send(b"Fixed", ENTER)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert headings(out) == ["# Issue #8: Fixed already"]
    assert "done: Copied context for Issue #8 (~" in err


def test_renders_the_full_comment_thread(github, github_checkout, tmp_path, terminal):
    github.add(f"{ISSUES}?state=open&per_page=100", [issue(1, "Crash")])
    github.add(f"{ISSUES}/1", issue(1, "Crash", body=None))
    github.add(
        f"{ISSUES}/1/comments?per_page=100",
        [comment("ana", "First"), comment("bo", "Second")],
        headers={"Link": f'<{github.link(ISSUES + "/1/comments?page=2")}>; rel="next"'},
    )
    github.add(f"{ISSUES}/1/comments?page=2", [comment("cy", "Third")])

    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("1/1")
    session.send(ENTER)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert out == (
        "<context>\n"
        "# Issue #1: Crash\n"
        "URL: https://github.com/octo/widget/issues/1\n\n"
        "## Description\n"
        "*No description provided.*\n\n"
        "## Discussion Thread\n\n"
        "### Comment 1 (@ana)\nFirst\n\n"
        "### Comment 2 (@bo)\nSecond\n\n"
        "### Comment 3 (@cy)\nThird\n"
        "</context>\n"
    )


def test_one_failing_issue_does_not_abort_the_others(
    github, github_checkout, tmp_path, terminal
):
    github.add(
        f"{ISSUES}?state=open&per_page=100", [issue(2, "Broken"), issue(1, "Fine")]
    )
    github.add(f"{ISSUES}/2", {"message": "Server Error"}, status=500)
    detail(github, 1, "Fine", body="ok")

    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("2/2")
    session.send(CTRL_A, ENTER)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert (
        "# Issue #2\n\n[Could not fetch issue details or comments]\n\n---\n\n"
        "# Issue #1: Fine\n"
    ) in out
    assert "error: could not fetch issue #2: GitHub API error (HTTP 500)" in err
    assert "done: Copied context for 2 issues" in err


def test_previews_the_issue_body(github, github_checkout, tmp_path, terminal):
    github.add(
        f"{ISSUES}?state=open&per_page=100",
        [issue(1, "Leak", body="Memory grows unbounded"), issue(2, "Empty")],
    )
    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("unbounded")
    session.send(b"Empty")
    session.wait_for("description")
    session.send(ESC)
    code, err, _, _ = session.finish()
    assert code == 0, err


def test_cancel_exits_quietly(github, github_checkout, tmp_path, terminal):
    github.add(f"{ISSUES}?state=open&per_page=100", [issue(1, "One")])
    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    session.wait_for("Select issues: ")
    session.wait_for("1/1")
    session.send(b"\t", ESC)
    code, err, out, _ = session.finish()
    assert code == 0, err
    assert out == ""
    assert err == "info: Fetching issues...\n"


def test_list_failure_exits_non_zero(github, github_checkout, tmp_path, terminal):
    github.add(
        f"{ISSUES}?state=open&per_page=100",
        {"message": "Bad credentials"},
        status=401,
    )
    session = terminal(github_checkout, "issues", env=github.env(tmp_path))
    code, err, out, _ = session.finish()
    assert code == 1
    assert out == ""
    assert "error: GitHub rejected the credentials (HTTP 401): Bad credentials" in err


def test_missing_credentials(github, github_checkout, tmp_path, focal):
    env = github.env(tmp_path, PATH="")
    del env["GH_TOKEN"]
    result = focal(github_checkout, env, "issues")
    assert result.returncode == 1
    assert b"error: no GitHub credentials found for github.com" in result.stderr
    assert b"set GH_TOKEN or GITHUB_TOKEN" in result.stderr
    assert github.requests == []


def test_missing_origin(github, github_checkout, tmp_path, focal):
    subprocess.run(
        ["git", "remote", "remove", "origin"], cwd=github_checkout, check=True
    )
    result = focal(github_checkout, github.env(tmp_path), "issues")
    assert result.returncode == 1
    assert b"error: this repository has no 'origin' remote" in result.stderr
