from unittest.mock import MagicMock

from focal import gh_pr_diff


def test_gh_pr_diff_happy_path(mocker, capsys):
    # 1. Setup the simulated subprocess response
    mock_run = mocker.patch("subprocess.run")

    # Simulate the metadata fetch and the diff fetch.
    mock_meta_response = MagicMock(
        returncode=0,
        stdout='{"title": "Fix memory leak", "body": "Cleared cache", "url": "https://fake"}',
    )
    mock_diff_response = MagicMock(returncode=0, stdout="+ added line\n- removed line")

    mock_run.side_effect = [mock_meta_response, mock_diff_response]

    # 2. Inject CLI arguments and execute
    mocker.patch("sys.argv", ["gh_pr_diff.py", "123"])
    gh_pr_diff.main()

    # 3. Capture the stdout and verify the data pipeline
    captured = capsys.readouterr()
    assert "# PR #123: Fix memory leak" in captured.out
    assert "```diff\n+ added line\n- removed line\n```" in captured.out
