from unittest.mock import MagicMock

from focal import gh_pr_diff


def test_gh_pr_diff_happy_path(mocker):
    mock_run = mocker.patch("focal.utils.subprocess.run")

    mock_meta_response = MagicMock(
        returncode=0,
        stdout='{"title": "Fix memory leak", "body": "Cleared cache", "url": "https://fake"}',
    )
    mock_diff_response = MagicMock(returncode=0, stdout="+ added line\n- removed line")

    mock_run.side_effect = [mock_meta_response, mock_diff_response]

    result = gh_pr_diff.get_pr_diff_context("123")

    assert "# PR #123: Fix memory leak" in result
    assert "```diff\n+ added line\n- removed line\n```" in result
