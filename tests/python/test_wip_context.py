from unittest.mock import MagicMock, patch

import pytest

from focal import wip_context


def test_run_git_success(mocker):
    mock_run = mocker.patch("subprocess.run")
    mock_run.return_value = MagicMock(returncode=0, stdout="  refs/heads/main  \n")

    code, out = wip_context.run_git(["rev-parse", "--abbrev-ref", "HEAD"])

    assert code == 0
    assert out == "refs/heads/main"  # Verifies the .strip() is applied
    mock_run.assert_called_once_with(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"], capture_output=True, text=True
    )


def test_is_priority_logic():
    assert wip_context.is_priority("pyproject.toml") is True
    assert wip_context.is_priority(".github/workflows/ci.yaml") is True
    assert wip_context.is_priority("focal/wip_context.py") is False


def test_is_noise_logic():
    assert wip_context.is_noise("package-lock.json") is True
    assert wip_context.is_noise("data/weights.h5") is True
    assert wip_context.is_noise("src/main.rs") is False


def test_run_git_failure():
    with patch("subprocess.run") as mock_run:
        mock_run.return_value = MagicMock(returncode=1, stderr="fatal error")
        with pytest.raises(SystemExit) as exc_info:
            wip_context.run_git(["bad"])

        assert "fatal error" in str(exc_info.value)


def test_resolve_base_branch_explicit():
    with patch("focal.wip_context.run_git") as mock_run_git:
        mock_run_git.return_value = (0, "")
        assert wip_context.resolve_base_branch("custom") == "custom"


def test_resolve_base_branch_dynamic():
    with patch("focal.wip_context.run_git") as mock_run_git:
        mock_run_git.side_effect = [(0, "refs/remotes/origin/main")]
        assert wip_context.resolve_base_branch(None) == "main"


def test_resolve_base_branch_fallback():
    with patch("focal.wip_context.run_git") as mock_run_git:
        # First call fails (symbolic-ref), second fails (main), third succeeds (master)
        mock_run_git.side_effect = [(1, ""), (1, ""), (0, "")]
        assert wip_context.resolve_base_branch(None) == "master"


def test_resolve_base_branch_failure():
    with patch("focal.wip_context.run_git") as mock_run_git:
        mock_run_git.return_value = (1, "")
        with pytest.raises(SystemExit):
            wip_context.resolve_base_branch(None)


def test_get_diff_for_files():
    with patch("focal.wip_context.run_git") as mock_run_git:
        mock_run_git.return_value = (0, "index 123..456\n+new line")

        blocks, _remaining, omitted = wip_context.get_diff_for_files(
            "base", ["file1.txt"], 100
        )

        assert len(blocks) == 1
        assert "file1.txt" in blocks[0]
        assert "+new line" in blocks[0]
        assert "index 123" not in blocks[0]
        assert omitted == 0


def test_get_diff_for_files_binary_or_empty():
    with patch("focal.wip_context.run_git") as mock_run_git:
        # Return empty for first file, binary string for second
        mock_run_git.side_effect = [(0, ""), (0, "Binary files a/b and c/d differ")]

        blocks, _remaining, _omitted = wip_context.get_diff_for_files(
            "base", ["f1.txt", "f2.bin"], 100
        )

        assert len(blocks) == 0


def test_main_success():
    with (
        patch("focal.wip_context.sys.argv", ["focal.wip_context", "main"]),
        patch("focal.wip_context.run_git") as mock_run_git,
        patch("focal.wip_context.sys.stdout.write") as mock_write,
    ):

        def mock_git(args, **kwargs):
            cmd = " ".join(args)
            if "rev-parse --is-inside-work-tree" in cmd:
                return 0, "true"
            if "rev-parse --verify main" in cmd:
                return 0, ""
            if "merge-base main HEAD" in cmd:
                return 0, "abcdef123456"
            if "rev-parse --short HEAD" in cmd:
                return 0, "1234567"
            if "status --porcelain" in cmd:
                return 0, "M  file.py"
            if "log --name-status" in cmd:
                return 0, "[abcdef] fix"
            if "diff --stat" in cmd:
                return 0, " 1 file changed"
            if "diff --name-only" in cmd:
                return 0, "file.py"
            if "diff -M abcdef123456..HEAD -- file.py" in cmd:
                return 0, "+diff"
            return 0, ""

        mock_run_git.side_effect = mock_git

        wip_context.main()

        output = mock_write.call_args[0][0]
        assert "# WIP Branch Context" in output
        assert "M  file.py" in output
        assert "[abcdef] fix" in output
        assert " 1 file changed" in output
        assert "+diff" in output
