from unittest.mock import patch

from focal.gh_release_context import get_release_context


def test_get_release_context_with_prs_and_commits():
    with (
        patch("focal.gh_release_context.resolve_base_branch") as mock_base,
        patch("focal.gh_release_context.run_gh_json") as mock_run_gh_json,
        patch("focal.gh_release_context.run_git") as mock_run_git,
    ):
        mock_base.return_value = "main"
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

        mock_run_git.return_value = (
            0,
            "a1b2c3d\tRevert change by @dependabot\tAlice\ne4f5g6h\tDependency update\tdependabot[bot]",
        )

        output = get_release_context("2023-01-01", "v1.0.0", "v0.9.0")

        assert "# Release Context: v1.0.0 to HEAD" in output
        assert "## PR #1: Fix bug" in output
        assert "**Author:** @user1" in output
        assert "**Labels:** bug" in output
        assert "Fixes #1" in output
        assert "<!-- hidden info -->" not in output

        assert "## Raw Commits" in output
        assert "* `a1b2c3d` - Revert change by @dependabot (@Alice)" in output
        assert "Dependency update" not in output


def test_get_release_context_no_prs_or_commits():
    with (
        patch("focal.gh_release_context.run_gh_json") as mock_run_gh_json,
        patch("focal.gh_release_context.run_git") as mock_run_git,
    ):
        mock_run_gh_json.return_value = []
        mock_run_git.return_value = (0, "")

        output = get_release_context("2023-01-01", "v1.0.0", "")

        assert "*No pull requests found matching the criteria.*" in output
        assert "## Raw Commits" not in output
