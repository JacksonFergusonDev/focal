from unittest.mock import MagicMock, patch

import pytest

from focal.gh_release_context import main


def test_main_success_with_prs_and_commits():
    with (
        patch(
            "focal.gh_release_context.sys.argv",
            ["focal.gh_release_context", "2023-01-01", "v1.0.0", "v0.9.0"],
        ),
        patch("focal.gh_release_context.run_gh_json") as mock_run_gh_json,
        patch("focal.gh_release_context.subprocess.run") as mock_run,
        patch("focal.gh_release_context.sys.stdout.write") as mock_write,
    ):
        mock_run_gh_json.return_value = [
            {
                "number": 1,
                "title": "Fix bug",
                "author": {"login": "user1"},
                "url": "https://github.com/org/repo/pull/1",
                "labels": [{"name": "bug"}],
                "body": "Fixes #1\n<!-- hidden info -->",
            }
        ]

        mock_git_log_res = MagicMock()
        mock_git_log_res.returncode = 0
        mock_git_log_res.stdout = "* `a1b2c3d` - Some commit (@user2)\n* `e4f5g6h` - Dependency update (@dependabot[bot])"
        mock_run.return_value = mock_git_log_res

        main()

        output = mock_write.call_args[0][0]

        assert "# Release Context: v1.0.0 to HEAD" in output
        assert "## PR #1: Fix bug" in output
        assert "**Author:** @user1" in output
        assert "**Labels:** bug" in output
        assert "Fixes #1" in output
        assert "<!-- hidden info -->" not in output

        assert "## Raw Commits" in output
        assert "* `a1b2c3d` - Some commit (@user2)" in output
        assert "Dependency update" not in output


def test_main_missing_args():
    with patch("focal.gh_release_context.sys.argv", ["focal.gh_release_context"]):
        with pytest.raises(SystemExit) as exc_info:
            main()

        assert (
            "Usage: python -m focal.gh_release_context <tag_date> <header_ref> <tag_ref>"
            in str(exc_info.value)
        )


def test_main_no_prs_or_commits():
    with (
        patch(
            "focal.gh_release_context.sys.argv",
            ["focal.gh_release_context", "2023-01-01", "v1.0.0", ""],
        ),
        patch("focal.gh_release_context.run_gh_json") as mock_run_gh_json,
        patch("focal.gh_release_context.sys.stdout.write") as mock_write,
    ):
        mock_run_gh_json.return_value = []

        main()

        output = mock_write.call_args[0][0]

        assert "*No pull requests found matching the criteria.*" in output
        assert "## Raw Commits" not in output
