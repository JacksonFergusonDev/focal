"""`pr-diff` contracts for the Rust CLI against the fake GitHub API.

The legacy command shells out to `gh` and fzf, so these run only against the Rust binary.
"""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()

pytestmark = pytest.mark.skipif(
    "FOCAL_TEST_BIN" not in os.environ, reason="legacy uses gh"
)

ESC, ENTER = b"\x1b", b"\r"
PULLS = "/repos/octo/widget/pulls"
PATCH = (
    "diff --git a/app.py b/app.py\n"
    "--- a/app.py\n+++ b/app.py\n"
    "@@ -1 +1 @@\n-old\n+new\n"
)


def header(request, name):
    return {k.lower(): v for k, v in request.headers.items()}[name.lower()]


def pull(number, title, body="Why this matters.", head="feature"):
    return {
        "number": number,
        "title": title,
        "body": body,
        "state": "open",
        "head": {"ref": head},
        "html_url": f"https://github.com/octo/widget/pull/{number}",
    }


def serve_pull(github, number, title, **fields):
    """The PR JSON then its patch: both live at the same path."""
    github.add(f"{PULLS}/{number}", pull(number, title, **fields))
    github.add(f"{PULLS}/{number}", raw=PATCH.encode())


def expected(number, title, body="Why this matters."):
    return (
        "<context>\n"
        f"# PR #{number}: {title}\n"
        f"URL: https://github.com/octo/widget/pull/{number}\n\n"
        "## Intent / Description\n"
        f"{body}\n\n"
        "## Diff\n"
        f"```diff\n{PATCH.strip()}\n```\n"
        "</context>\n"
    )


@pytest.fixture
def env(github, tmp_path):
    return github.env(tmp_path, FOCAL_OMIT_WORKSPACE="1")


def test_explicit_number(github, github_checkout, env, focal):
    serve_pull(github, 7, "Add widget")
    result = focal(github_checkout, env, "pr-diff", "7")
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode() == expected(7, "Add widget")
    assert result.stderr.decode().startswith("done: Copied context and diff for PR #7")
    accepts = [header(r, "Accept") for r in github.requests]
    assert accepts == ["application/vnd.github+json", "application/vnd.github.diff"]
    assert {header(r, "Authorization") for r in github.requests} == {
        "Bearer test-token"
    }


def test_explicit_url_and_empty_body(github, github_checkout, env, focal):
    serve_pull(github, 12, "Docs", body=None)
    result = focal(
        github_checkout,
        env,
        "pr-diff",
        "https://github.com/octo/widget/pull/12/files",
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode() == expected(12, "Docs", "*No description provided.*")


@pytest.mark.parametrize("where", ["unrelated checkout", "outside any repository"])
def test_url_for_another_repository(
    github, github_checkout, tmp_path, env, focal, where
):
    other = "/repos/other/lib/pulls/42"
    github.add(
        other,
        {
            **pull(42, "Upstream fix"),
            "html_url": "https://github.com/other/lib/pull/42",
        },
    )
    github.add(other, raw=PATCH.encode())
    cwd = github_checkout if where == "unrelated checkout" else tmp_path
    result = focal(cwd, env, "pr-diff", "https://github.com/other/lib/pull/42/files")
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode().startswith("<context>\n# PR #42: Upstream fix\n")
    assert [r.path for r in github.requests] == [other, other]


def test_branch_name_containing_pull_is_not_a_url(github, github_checkout, env, focal):
    github.add(
        f"{PULLS}?head=octo:release/pull/3&state=all&per_page=100",
        [pull(9, "Release")],
    )
    serve_pull(github, 9, "Release")
    result = focal(github_checkout, env, "pr-diff", "release/pull/3")
    assert result.returncode == 0, result.stderr
    assert "# PR #9: Release" in result.stdout.decode()


def test_explicit_branch_name(github, github_checkout, env, focal):
    github.add(
        f"{PULLS}?head=octo:feature/login&state=all&per_page=100",
        [pull(3, "Login")],
    )
    serve_pull(github, 3, "Login")
    result = focal(github_checkout, env, "pr-diff", "feature/login")
    assert result.returncode == 0, result.stderr
    assert "# PR #3: Login" in result.stdout.decode()


def test_current_branch_pull_request(github, github_checkout, env, focal):
    closed = {**pull(4, "Old attempt"), "state": "closed"}
    github.add(
        f"{PULLS}?head=octo:main&state=all&per_page=100",
        [closed, pull(5, "Current")],
    )
    serve_pull(github, 5, "Current")
    result = focal(github_checkout, env, "pr-diff")
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode() == expected(5, "Current")


def test_current_branch_uses_upstream_name(github, github_checkout, env, focal):
    subprocess.run(
        ["git", "config", "branch.main.merge", "refs/heads/topic"],
        cwd=github_checkout,
        check=True,
    )
    github.add(f"{PULLS}?head=octo:topic&state=all&per_page=100", [pull(6, "Topic")])
    serve_pull(github, 6, "Topic")
    result = focal(github_checkout, env, "pr-diff")
    assert result.returncode == 0, result.stderr
    assert "# PR #6: Topic" in result.stdout.decode()


@pytest.mark.parametrize("lookup", ["none", "error"])
def test_lookup_falls_back_to_the_picker(
    github, github_checkout, env, terminal, lookup
):
    query = f"{PULLS}?head=octo:main&state=all&per_page=100"
    if lookup == "none":
        github.add(query, [])
    else:
        github.add(query, {"message": "boom"}, status=500)
    github.add(
        f"{PULLS}?state=open&per_page=30",
        [pull(21, "Refactor", head="cleanup"), pull(20, "Add cache", head="cache")],
    )
    serve_pull(github, 20, "Add cache")

    session = terminal(github_checkout, "pr-diff", env=env)
    session.wait_for("Select PR: ")
    session.wait_for("[cleanup] Refactor")
    session.send(b"cache", ENTER)
    code, err, out, _ = session.finish()

    assert code == 0, err
    assert out == expected(20, "Add cache")
    assert "boom" not in err


def test_picker_cancel_exits_quietly(github, github_checkout, env, terminal):
    github.add(f"{PULLS}?head=octo:main&state=all&per_page=100", [])
    github.add(f"{PULLS}?state=open&per_page=30", [pull(1, "Only")])
    session = terminal(github_checkout, "pr-diff", env=env)
    session.wait_for("[feature] Only")
    session.send(ESC)
    code, err, out, _ = session.finish()
    assert (code, err, out) == (0, "", "")


def test_no_terminal_for_the_fallback(github, github_checkout, env):
    github.add(f"{PULLS}?head=octo:main&state=all&per_page=100", [])
    result = subprocess.run(
        [CLI, "pr-diff"],
        cwd=github_checkout,
        env=env,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        check=False,
    )
    assert result.returncode == 1
    assert b"no pull request found for the current branch" in result.stderr
    assert b"focal pr-diff <number>" in result.stderr


def test_missing_pull_request(github, github_checkout, env, focal):
    github.add(f"{PULLS}/404", {"message": "Not Found"}, status=404)
    result = focal(github_checkout, env, "pr-diff", "404")
    assert result.returncode == 1
    assert b"error: not found on GitHub (HTTP 404)" in result.stderr
    assert result.stdout == b""


def test_patch_failure(github, github_checkout, env, focal):
    github.add(f"{PULLS}/8", pull(8, "Huge"))
    github.add(f"{PULLS}/8", {"message": "diff too large"}, status=406)
    result = focal(github_checkout, env, "pr-diff", "8")
    assert result.returncode == 1
    assert b"GitHub API error (HTTP 406): diff too large" in result.stderr
    assert b"verify that the PR exists and you have access" in result.stderr


def test_missing_credentials(github, github_checkout, env, focal):
    env = {**env, "PATH": ""}
    del env["GH_TOKEN"]
    result = focal(github_checkout, env, "pr-diff", "7")
    assert result.returncode == 1
    assert b"error: no GitHub credentials found for github.com" in result.stderr
    assert github.requests == []


def test_diff_that_is_not_utf8_is_decoded_lossily(github, github_checkout, env, focal):
    github.add(f"{PULLS}/3", pull(3, "Latin-1"))
    github.add(f"{PULLS}/3", raw=b"diff --git a/l b/l\n+caf\xe9\n")
    result = focal(github_checkout, env, "pr-diff", "3")
    assert result.returncode == 0, result.stderr
    assert "+caf�" in result.stdout.decode()


def test_too_many_requests_is_a_rate_limit(github, github_checkout, env, focal):
    github.add(
        f"{PULLS}/4",
        {"message": "slow down"},
        status=429,
        headers={"Retry-After": "60"},
    )
    result = focal(github_checkout, env, "pr-diff", "4")
    assert result.returncode == 1
    stderr = result.stderr.decode()
    assert "rate limit" in stderr
    assert "wait 60 seconds" in stderr


def test_credentials_in_origin_are_not_printed(github, github_checkout, env, focal):
    subprocess.run(
        [
            "git",
            "remote",
            "set-url",
            "origin",
            "https://x-access-token:ghp_SECRET@github.com/",
        ],
        cwd=github_checkout,
        check=True,
    )
    result = focal(github_checkout, env, "pr-diff", "1")
    assert result.returncode == 1
    assert b"could not read a GitHub owner and repository" in result.stderr
    assert b"ghp_SECRET" not in result.stderr
    assert not github.requests
