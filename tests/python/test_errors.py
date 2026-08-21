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
