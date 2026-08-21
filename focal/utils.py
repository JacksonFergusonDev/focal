"""Utility functions for executing GitHub CLI commands and parsing output."""

import json
import subprocess
from typing import Any

from focal.errors import die, print_error


def run_gh_json(args: list[str], exit_on_error: bool = True) -> Any:
    """Executes a gh CLI command that outputs JSON and returns the parsed payload.

    Args:
        args: The arguments to pass to the `gh` command.
        exit_on_error: If True, exits the program on failure. Otherwise, prints
            the error to stderr and returns None.

    Returns:
        The parsed JSON output, or None if the command failed and exit_on_error
        is False.

    Raises:
        SystemExit: If the command fails or returns invalid JSON and exit_on_error
            is True.
    """
    res = subprocess.run(["gh", *args], capture_output=True, text=True)
    if res.returncode != 0:
        err_msg = f"Error executing gh {' '.join(args)}: {res.stderr.strip()}"
        if exit_on_error:
            die(err_msg)
        else:
            print_error(err_msg)
            return None

    try:
        return json.loads(res.stdout)
    except json.JSONDecodeError:
        err_msg = f"Failed to parse JSON output from gh {' '.join(args)}"
        if exit_on_error:
            die(err_msg)
        else:
            print_error(err_msg)
            return None
