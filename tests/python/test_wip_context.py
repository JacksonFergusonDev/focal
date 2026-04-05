from unittest.mock import MagicMock

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
