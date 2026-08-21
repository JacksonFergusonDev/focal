"""Centralized error, warning, and diagnostic reporting for focal."""

import os
import sys
from typing import NoReturn


def _should_use_color() -> bool:
    """Determines whether ANSI color codes should be enabled for stderr."""
    return (
        sys.stderr.isatty()
        and "NO_COLOR" not in os.environ
        and os.environ.get("TERM") != "dumb"
    )


def format_error(message: str, hint: str | None = None) -> str:
    """Formats an error message and optional hint with ANSI colors.

    Args:
        message: The primary error description.
        hint: Optional actionable advice or syntax hint.

    Returns:
        A formatted error string ready for stderr or sys.exit.
    """
    use_color = _should_use_color()
    bold = "\033[1m" if use_color else ""
    dim = "\033[2m" if use_color else ""
    red = "\033[31m" if use_color else ""
    cyan = "\033[36m" if use_color else ""
    reset = "\033[0m" if use_color else ""

    lines = [f"{bold}{red}error:{reset} {message}"]
    if hint:
        lines.append(f"  {dim}{cyan}hint:{reset} {hint}")
    return "\n".join(lines)


def format_warning(message: str, hint: str | None = None) -> str:
    """Formats a warning message and optional hint with ANSI colors.

    Args:
        message: The primary warning description.
        hint: Optional actionable advice or context.

    Returns:
        A formatted warning string ready for stderr.
    """
    use_color = _should_use_color()
    bold = "\033[1m" if use_color else ""
    dim = "\033[2m" if use_color else ""
    yellow = "\033[33m" if use_color else ""
    cyan = "\033[36m" if use_color else ""
    reset = "\033[0m" if use_color else ""

    lines = [f"{bold}{yellow}warning:{reset} {message}"]
    if hint:
        lines.append(f"  {dim}{cyan}hint:{reset} {hint}")
    return "\n".join(lines)


def print_error(message: str, hint: str | None = None) -> None:
    """Prints a formatted error message and optional hint to stderr."""
    print(format_error(message, hint), file=sys.stderr)


def print_warning(message: str, hint: str | None = None) -> None:
    """Prints a formatted warning message and optional hint to stderr."""
    print(format_warning(message, hint), file=sys.stderr)


def die(message: str, hint: str | None = None) -> NoReturn:
    """Exits the program with returncode 1, printing formatted error to stderr."""
    sys.exit(format_error(message, hint))
