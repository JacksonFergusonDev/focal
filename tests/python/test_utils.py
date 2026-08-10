from unittest.mock import MagicMock, patch

import pytest

from focal.utils import run_gh_json


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
        patch("focal.utils.sys.stderr", new_callable=MagicMock),
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

        assert "Error: Failed to parse JSON output from gh test" in str(exc_info.value)


def test_run_gh_json_decode_error_no_exit():
    with (
        patch("focal.utils.subprocess.run") as mock_run,
        patch("focal.utils.sys.stderr", new_callable=MagicMock),
    ):
        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = "not valid json"
        mock_run.return_value = mock_result

        result = run_gh_json(["test"], exit_on_error=False)

        assert result is None
