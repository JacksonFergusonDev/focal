from unittest.mock import MagicMock, patch

import pytest

from focal.utils import run_gh, run_gh_json, run_git


def test_run_gh_json_success():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = '{"key": "value"}'
        mock_run.return_value = mock_result

        result = run_gh_json(["test", "arg"])

        assert result == {"key": "value"}
        mock_run.assert_called_once_with(
            ["gh", "test", "arg"], capture_output=True, text=True
        )


def test_run_gh_json_command_failure_exit():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock()
        mock_result.returncode = 1
        mock_result.stderr = "Command failed"
        mock_run.return_value = mock_result

        with pytest.raises(SystemExit) as exc_info:
            run_gh_json(["test"])

        assert "Error executing gh test: Command failed" in str(exc_info.value)


def test_run_gh_json_command_failure_no_exit():
    with (
        patch("focal.utils.subprocess.run") as mock_run,
        patch("focal.errors.sys.stderr", new_callable=MagicMock),
    ):
        mock_result = MagicMock()
        mock_result.returncode = 1
        mock_result.stderr = "Command failed"
        mock_run.return_value = mock_result

        result = run_gh_json(["test"], exit_on_error=False)

        assert result is None


def test_run_gh_json_decode_error_exit():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = "not valid json"
        mock_run.return_value = mock_result

        with pytest.raises(SystemExit) as exc_info:
            run_gh_json(["test"])

        assert "Failed to parse JSON output from gh test" in str(exc_info.value)


def test_run_gh_json_decode_error_no_exit():
    with (
        patch("focal.utils.subprocess.run") as mock_run,
        patch("focal.errors.sys.stderr", new_callable=MagicMock),
    ):
        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = "not valid json"
        mock_run.return_value = mock_result

        result = run_gh_json(["test"], exit_on_error=False)

        assert result is None


def test_run_gh_success():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock(returncode=0, stdout="diff output\n")
        mock_run.return_value = mock_result

        code, out = run_gh(["pr", "diff", "123"])

        assert code == 0
        assert out == "diff output"
        mock_run.assert_called_once_with(
            ["gh", "pr", "diff", "123"], capture_output=True, text=True
        )


def test_run_gh_failure_exit():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock(returncode=1, stderr="not found")
        mock_run.return_value = mock_result

        with pytest.raises(SystemExit) as exc_info:
            run_gh(["pr", "diff", "999"])

        assert "Error executing gh pr diff 999: not found" in str(exc_info.value)


def test_run_gh_failure_no_exit():
    with (
        patch("focal.utils.subprocess.run") as mock_run,
        patch("focal.errors.sys.stderr", new_callable=MagicMock),
    ):
        mock_result = MagicMock(returncode=1, stderr="not found")
        mock_run.return_value = mock_result

        code, _ = run_gh(["pr", "diff", "999"], exit_on_error=False)

        assert code == 1


def test_run_git_success():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock(returncode=0, stdout="main\n")
        mock_run.return_value = mock_result

        code, out = run_git(["rev-parse", "--abbrev-ref", "HEAD"])

        assert code == 0
        assert out == "main"


def test_run_git_failure_exit():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock(returncode=1, stderr="fatal: not a git repo")
        mock_run.return_value = mock_result

        with pytest.raises(SystemExit) as exc_info:
            run_git(["status"])

        assert "Git command failed: git status" in str(exc_info.value)


def test_run_git_failure_no_check():
    with patch("focal.utils.subprocess.run") as mock_run:
        mock_result = MagicMock(returncode=1, stdout="", stderr="error")
        mock_run.return_value = mock_result

        code, out = run_git(["status"], check=False)

        assert code == 1
        assert out == ""


def test_bot_helpers():
    from focal.utils import build_bot_exclusion_query, is_bot_author

    query = build_bot_exclusion_query()
    assert "-author:app/renovate" in query
    assert "-author:dependabot" in query
    assert "-author:github-actions" in query

    assert is_bot_author("dependabot[bot]") is True
    assert is_bot_author("Commit message (@dependabot)") is True
    assert is_bot_author("renovate[bot]") is True
    assert is_bot_author("developer") is False


def test_resolve_base_branch_with_slashes():
    from focal.utils import resolve_base_branch

    with patch("focal.utils.run_git") as mock_run_git:
        mock_run_git.side_effect = [(0, "refs/remotes/origin/release/v1.0\n")]
        assert resolve_base_branch(None) == "release/v1.0"


def test_resolve_base_branch_explicit():
    from focal.utils import resolve_base_branch

    with patch("focal.utils.run_git") as mock_run_git:
        mock_run_git.return_value = (0, "")
        assert resolve_base_branch("custom-branch") == "custom-branch"


def test_resolve_base_branch_fallbacks():
    from focal.utils import resolve_base_branch

    with patch("focal.utils.run_git") as mock_run_git:
        # symbolic-ref fails, main fails, master succeeds
        mock_run_git.side_effect = [(1, ""), (1, ""), (0, "")]
        assert resolve_base_branch(None) == "master"
