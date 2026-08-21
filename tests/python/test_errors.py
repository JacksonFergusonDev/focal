import os
from unittest.mock import patch

import pytest

from focal import errors


def test_format_error_no_color():
    with patch.dict(os.environ, {"NO_COLOR": "1"}):
        out = errors.format_error("something failed")
        assert out == "error: something failed"

        out_with_hint = errors.format_error("something failed", hint="try this")
        assert out_with_hint == "error: something failed\n  hint: try this"


def test_format_warning_no_color():
    with patch.dict(os.environ, {"NO_COLOR": "1"}):
        out = errors.format_warning("be careful")
        assert out == "warning: be careful"

        out_with_hint = errors.format_warning("be careful", hint="watch out")
        assert out_with_hint == "warning: be careful\n  hint: watch out"


def test_format_error_with_color():
    with (
        patch.dict(os.environ, {}, clear=True),
        patch("sys.stderr.isatty", return_value=True),
    ):
        out = errors.format_error("bad thing", hint="fix it")
        assert "\033[31m" in out
        assert "\033[36m" in out
        assert "bad thing" in out
        assert "fix it" in out


def test_die_raises_system_exit():
    with patch.dict(os.environ, {"NO_COLOR": "1"}):
        with pytest.raises(SystemExit) as exc_info:
            errors.die("fatal problem", hint="check logs")
        assert "error: fatal problem" in str(exc_info.value)
        assert "hint: check logs" in str(exc_info.value)


def test_print_error_and_warning(capsys):
    with patch.dict(os.environ, {"NO_COLOR": "1"}):
        errors.print_error("an error", hint="an error hint")
        errors.print_warning("a warning", hint="a warning hint")

        captured = capsys.readouterr()
        assert "error: an error" in captured.err
        assert "hint: an error hint" in captured.err
        assert "warning: a warning" in captured.err
        assert "hint: a warning hint" in captured.err


def test_cross_runtime_parity_with_bash():
    """Asserts that Python and Bash implementations output identical diagnostic text."""
    import subprocess
    from pathlib import Path

    repo_root = Path(__file__).parent.parent.parent
    core_sh = repo_root / "lib" / "core.sh"

    # Test error without hint
    py_err = errors.format_error("something failed")
    bash_err = subprocess.run(
        [
            "bash",
            "-c",
            f"source '{core_sh}'; status_error 'something failed'",
        ],
        capture_output=True,
        text=True,
        env={"NO_COLOR": "1", "PATH": os.environ.get("PATH", "")},
    ).stderr.strip()
    assert py_err == bash_err

    # Test error with hint
    py_err_hint = errors.format_error("something failed", hint="try this")
    bash_err_hint = subprocess.run(
        [
            "bash",
            "-c",
            f"source '{core_sh}'; status_error 'something failed' && status_hint 'try this'",
        ],
        capture_output=True,
        text=True,
        env={"NO_COLOR": "1", "PATH": os.environ.get("PATH", "")},
    ).stderr.strip()
    assert py_err_hint == bash_err_hint

    # Test warning with hint
    py_warn_hint = errors.format_warning("be careful", hint="watch out")
    bash_warn_hint = subprocess.run(
        [
            "bash",
            "-c",
            f"source '{core_sh}'; status_warn 'be careful' && status_hint 'watch out'",
        ],
        capture_output=True,
        text=True,
        env={"NO_COLOR": "1", "PATH": os.environ.get("PATH", "")},
    ).stderr.strip()
    assert py_warn_hint == bash_warn_hint
