from unittest.mock import MagicMock, patch

import pytest

from focal.gh_ci_fail import main


def test_gh_ci_fail_main_success():
    with (
        patch("focal.gh_ci_fail.sys.argv", ["focal.gh_ci_fail", "123"]),
        patch("focal.gh_ci_fail.subprocess.run") as mock_run,
        patch("focal.gh_ci_fail.run_gh_json") as mock_run_gh_json,
        patch("builtins.print") as mock_print,
    ):
        mock_log_res = MagicMock()
        mock_log_res.returncode = 0
        mock_log_res.stdout = "failed log content"
        mock_run.return_value = mock_log_res

        mock_run_gh_json.return_value = {"name": "Test Run", "displayTitle": "PR Fix"}

        main()

        mock_run.assert_called_once_with(
            ["gh", "run", "view", "123", "--log-failed"], capture_output=True, text=True
        )
        mock_run_gh_json.assert_called_once_with(
            ["run", "view", "123", "--json", "name,displayTitle"], exit_on_error=False
        )

        mock_print.assert_called_once_with(
            "# CI Failure Context: Test Run - PR Fix\n\n```text\n[error logs]\nfailed log content\n```"
        )


def test_gh_ci_fail_main_missing_args():
    with patch("focal.gh_ci_fail.sys.argv", ["focal.gh_ci_fail"]):
        with pytest.raises(SystemExit) as exc_info:
            main()

        assert "missing run_id argument" in str(exc_info.value)


def test_gh_ci_fail_main_log_failure():
    with (
        patch("focal.gh_ci_fail.sys.argv", ["focal.gh_ci_fail", "123"]),
        patch("focal.gh_ci_fail.subprocess.run") as mock_run,
    ):
        mock_log_res = MagicMock()
        mock_log_res.returncode = 1
        mock_log_res.stderr = "api error"
        mock_run.return_value = mock_log_res

        with pytest.raises(SystemExit) as exc_info:
            main()

        assert "error fetching logs: api error" in str(exc_info.value)


def test_gh_ci_fail_filter_logs():
    from focal.gh_ci_fail import filter_logs

    # Test 1: Simple prefix stripping
    raw_logs = "Job Name\tStep Name\t2026-08-10T19:09:40.6560982Z Some logs"
    expected = "Some logs"
    assert filter_logs(raw_logs) == expected

    # Test 2: Collapsing successful group
    raw_logs = (
        "Job Name\tStep Name\t2026-08-10T19:09:40.123Z ##[group]Runner Image Provisioner\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.124Z doing things\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.125Z ##[endgroup]"
    )
    expected = "> [Group completed successfully: Runner Image Provisioner]"
    assert filter_logs(raw_logs) == expected

    # Test 3: Keeping failed group
    raw_logs = (
        "Job Name\tStep Name\t2026-08-10T19:09:40.123Z ##[group]Build Project\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.124Z ##[error]Process completed with exit code 1.\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.125Z ##[endgroup]"
    )
    expected = "##[group]Build Project\n##[error]Process completed with exit code 1.\n##[endgroup]"
    assert filter_logs(raw_logs) == expected

    # Test 4: Ignoring boilerplate
    raw_logs = (
        "Job Name\tStep Name\t2026-08-10T19:09:40.123Z Download action repository 'actions/checkout'\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.124Z Secret source: Actions\n"
        "Job Name\tStep Name\t2026-08-10T19:09:40.125Z Real error log"
    )
    expected = "Real error log"
    assert filter_logs(raw_logs) == expected

    # Test 5: Error context windowing
    lines = []
    for i in range(60):
        lines.append(f"Job Name\tStep Name\t2026-08-10T19:09:40.125Z Line {i}")
    lines[15] = "Job Name\tStep Name\t2026-08-10T19:09:40.125Z ##[error]Oops 1"
    lines[50] = "Job Name\tStep Name\t2026-08-10T19:09:40.125Z ##[error]Oops 2"

    raw_logs = "\n".join(lines)
    result = filter_logs(raw_logs).splitlines()

    assert "Line 0" in result
    assert "Line 25" in result
    assert "..." in result
    assert "Line 30" not in result
    assert "Line 35" in result
    assert "Line 59" in result

    # Test 6: Stripping ANSI escape codes
    raw_logs = "Job Name\tStep Name\t2026-08-10T19:09:40.125Z \x1b[34m==>\x1b[0m \x1b[1mHomebrew is cool\x1b[0m"
    expected = "==> Homebrew is cool"
    assert filter_logs(raw_logs) == expected
