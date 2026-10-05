import json
import subprocess
import sys
from unittest.mock import MagicMock, patch

import pytest

from focal.release_github import ReleaseGitHub
from focal.release_output import limit_release_output
from focal.release_prs import collect_pr_associations


def process_result(stdout='{"ok":true}', stderr="", code=0):
    process = MagicMock(returncode=code)
    process.communicate.return_value = (stdout, stderr)
    return process


def test_request_is_noninteractive_and_bounded():
    process = process_result()
    with patch("focal.release_github.subprocess.Popen", return_value=process) as spawn:
        assert ReleaseGitHub(request_timeout=3).request(["api", "test"]) == {"ok": True}
    assert spawn.call_args.kwargs["start_new_session"] is True
    assert spawn.call_args.kwargs["stdin"] == subprocess.DEVNULL
    assert spawn.call_args.kwargs["env"]["GH_PROMPT_DISABLED"] == "1"
    process.communicate.assert_called_once_with(timeout=3)


def test_timeout_kills_process_group_and_skips_remaining_requests():
    process = process_result()
    process.pid = 1234
    process.communicate.side_effect = [subprocess.TimeoutExpired("gh", 1), ("", "")]
    with (
        patch("focal.release_github.subprocess.Popen", return_value=process) as spawn,
        patch("focal.release_github.os.killpg") as kill,
    ):
        client = ReleaseGitHub(request_timeout=1)
        assert client.request(["api", "test"]) is None
        assert client.request(["api", "next"]) is None
    kill.assert_called_once()
    assert kill.call_args.args[0] == 1234
    spawn.assert_called_once()
    assert "timed out" in " ".join(client.notes)


def test_total_budget_caps_request_timeout_and_stops_before_next():
    process = process_result()
    with (
        patch("focal.release_github.time.monotonic", side_effect=[100, 108, 111]),
        patch("focal.release_github.subprocess.Popen", return_value=process) as spawn,
    ):
        client = ReleaseGitHub(request_timeout=5, github_timeout=10)
        client.request(["api", "test"])
        assert client.request(["api", "next"]) is None
    process.communicate.assert_called_once_with(timeout=2)
    spawn.assert_called_once()
    assert "budget" in " ".join(client.notes)


@pytest.mark.parametrize(
    "stderr",
    [
        "API rate limit exceeded (HTTP 403)",
        "secondary rate limit",
        "HTTP 429",
        "Bad credentials (HTTP 401)",
        "dial tcp: connection failed",
        "HTTP 503",
    ],
)
def test_rate_auth_and_network_failures_open_circuit(stderr):
    with patch(
        "focal.release_github.subprocess.Popen",
        return_value=process_result("", stderr, 1),
    ) as spawn:
        client = ReleaseGitHub()
        assert client.request(["api", "test"]) is None
        assert client.request(["api", "next"]) is None
    assert client.stopped
    spawn.assert_called_once()


def test_graphql_rate_limit_error_even_with_successful_process():
    payload = {"errors": [{"type": "RATE_LIMITED", "message": "limit exceeded"}]}
    with patch(
        "focal.release_github.subprocess.Popen",
        return_value=process_result(json.dumps(payload)),
    ):
        client = ReleaseGitHub()
        assert client.request(["api", "graphql"]) is None
    assert client.stopped


def test_graphql_partial_data_is_kept():
    payload = {
        "data": {"repository": {"c0": None}},
        "errors": [{"type": "NOT_FOUND", "message": "unpublished commit"}],
    }
    with patch(
        "focal.release_github.subprocess.Popen",
        return_value=process_result(json.dumps(payload), code=1),
    ):
        client = ReleaseGitHub()
        assert client.request(["api", "graphql"]) == payload
    assert not client.stopped
    assert "request failed" in " ".join(client.notes)


def test_keyboard_interrupt_cleans_up_process_group():
    process = process_result()
    process.communicate.side_effect = [KeyboardInterrupt, ("", "")]
    with (
        patch("focal.release_github.subprocess.Popen", return_value=process),
        patch("focal.release_github.os.killpg") as kill,
        pytest.raises(KeyboardInterrupt),
    ):
        ReleaseGitHub().request(["api", "test"])
    kill.assert_called_once()


def connection(nodes=(), more=False):
    return {
        "associatedPullRequests": {
            "nodes": list(nodes),
            "pageInfo": {"hasNextPage": more},
        }
    }


def graphql_pr():
    return {
        "number": 1,
        "title": "Feature",
        "body": "Intent",
        "url": "https://github.com/org/repo/pull/1",
        "mergedAt": "2026-01-01",
        "author": {"login": "alice", "__typename": "User"},
        "mergeCommit": {"oid": "a"},
        "labels": {"nodes": [{"name": "feature"}], "pageInfo": {"hasNextPage": False}},
    }


def test_45_commits_use_three_requests():
    client = ReleaseGitHub()
    responses = [
        {"data": {"repository": {f"c{i}": connection() for i in range(count)}}}
        for count in (20, 20, 5)
    ]
    with patch.object(client, "request", side_effect=responses) as request:
        result = collect_pr_associations(
            "org/repo", [str(i) for i in range(45)], client
        )
    assert request.call_count == 3
    assert len(result) == 45
    assert all(value == [] for value in result.values())


def test_missing_commit_is_unknown_and_other_batch_data_survives():
    client = ReleaseGitHub()
    payload = {"data": {"repository": {"c0": connection([graphql_pr()]), "c1": None}}}
    with patch.object(client, "request", return_value=payload):
        result = collect_pr_associations("org/repo", ["a", "b"], client)
    assert result["a"] is not None
    assert result["a"][0]["number"] == 1
    assert result["a"][0]["merge_commit_sha"] == "a"
    assert result["b"] is None


def test_overflow_uses_rest_pagination_without_truncation():
    client = ReleaseGitHub()
    payload = {"data": {"repository": {"c0": connection(more=True)}}}
    with patch.object(
        client,
        "request",
        side_effect=[payload, [{"number": i} for i in range(100)], [{"number": 100}]],
    ) as request:
        result = collect_pr_associations("org/repo", ["a"], client)
    assert result["a"] is not None
    assert len(result["a"]) == 101
    assert "page=2" in request.call_args_list[-1].args[0][1]


def test_stopped_client_prevents_rest_fallback_and_later_batches():
    client = ReleaseGitHub()

    def rate_limit(args):
        client._stop("rate limit reached")
        return

    with patch.object(client, "request", side_effect=rate_limit) as request:
        result = collect_pr_associations(
            "org/repo", [str(i) for i in range(45)], client
        )
    request.assert_called_once()
    assert all(value is None for value in result.values())


def test_successful_batch_exhausting_quota_keeps_data_and_stops_next_request():
    payload = {
        "data": {
            "repository": {"c0": connection()},
            "rateLimit": {"remaining": 0, "resetAt": "2026-09-30T23:00:00Z"},
        }
    }
    with patch(
        "focal.release_github.subprocess.Popen",
        return_value=process_result(json.dumps(payload)),
    ) as spawn:
        client = ReleaseGitHub()
        assert client.request(["api", "graphql"]) == payload
        assert client.request(["api", "next"]) is None
    assert client.stopped
    spawn.assert_called_once()


def test_output_budget_retains_provenance_and_completeness():
    content = (
        "# Release Context\nBase: abc\nHead: def\n\n```diff\n"
        + "lots of detail\n" * 10000
        + "```\n\n## Completeness Notes\n\n* Collected 259 commits; 75 merged PRs.\n"
    )
    output = limit_release_output(content, 1000)
    assert len(output) <= 4000
    assert "Base: abc" in output
    assert "Head: def" in output
    assert "Collected 259 commits" in output
    assert "payload is incomplete" in output
    assert output.count("```") % 2 == 0
    assert limit_release_output(content, 0) == content


def test_output_budget_cannot_be_defeated_by_large_diagnostics():
    content = (
        "# Release Context\n"
        + "detail\n" * 10000
        + "\n## Completeness Notes\n"
        + "* warning\n" * 10000
    )
    assert len(limit_release_output(content, 1000)) <= 4000


def test_output_budget_preserves_release_impact_as_well_as_counts():
    content = (
        "# Release Context\nHead: def\n"
        + "PR detail\n" * 10000
        + "\n## Release Impact\n\n```text\napp.py | 3 +++\n1 file changed\n```\n\n## Completeness Notes\n\n* Collected 259 commits.\n"
    )
    output = limit_release_output(content, 1000)
    assert len(output) <= 4000
    assert "app.py | 3 +++" in output
    assert "Collected 259 commits" in output


def test_real_hanging_request_is_terminated(tmp_path, monkeypatch):
    executable = tmp_path / "gh"
    executable.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(10)\n")
    executable.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path))
    client = ReleaseGitHub(request_timeout=0.2)
    assert client.request(["api", "test"]) is None
    assert client.stopped
    assert "timed out" in " ".join(client.notes)
