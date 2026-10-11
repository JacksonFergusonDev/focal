"""Utility functions for executing external CLI commands and parsing output."""

import json
import re
import subprocess
from typing import Any

from focal.errors import die, print_error

DEPENDENCY_BOT_USERS = [
    "app/renovate",
    "dependabot",
    "github-actions",
]

BOT_AUTHORS = [
    "dependabot[bot]",
    "dependabot",
    "renovate[bot]",
    "renovate",
    "github-actions[bot]",
    "github-actions",
]


def md_fence(content: str) -> str:
    """Builds a backtick fence that content's own backtick runs cannot close.

    Args:
        content: The text that will be wrapped in a fenced code block.

    Returns:
        A run of backticks at least three long and longer than any run in content.
    """
    longest = max((len(run) for run in re.findall(r"`+", content)), default=0)
    return "`" * max(3, longest + 1)


def fenced(content: str, lang: str = "") -> str:
    """Wraps content in a Markdown code block that nested fences cannot break.

    Args:
        content: The text to wrap.
        lang: Optional info string (language) for the opening fence.

    Returns:
        The fenced code block, without a trailing newline.
    """
    fence = md_fence(content)
    return f"{fence}{lang}\n{content}\n{fence}"


def build_bot_exclusion_query() -> str:
    """Builds a GitHub search query substring to exclude dependency bots.

    Returns:
        A space-delimited string of `-author:<bot>` filters.
    """
    return " ".join(f"-author:{bot}" for bot in DEPENDENCY_BOT_USERS)


def is_bot_author(author_label: str) -> bool:
    """Checks whether an author string matches known dependency or CI bots.

    Args:
        author_label: The author name or string containing author metadata.

    Returns:
        True if the author matches a known bot, False otherwise.
    """
    for bot in BOT_AUTHORS:
        if (
            f"(@{bot})" in author_label
            or f"@{bot}" in author_label
            or author_label == bot
        ):
            return True
    return False


def run_gh(args: list[str], exit_on_error: bool = True) -> tuple[int, str]:
    """Executes a gh CLI command and returns its exit code and standard output.

    Args:
        args: The arguments to pass to the `gh` command.
        exit_on_error: If True, exits the program on failure.

    Returns:
        The return code and stripped standard output.

    Raises:
        SystemExit: If the command fails and exit_on_error is True.
    """
    res = subprocess.run(["gh", *args], capture_output=True, text=True)
    if res.returncode != 0:
        err_msg = f"Error executing gh {' '.join(args)}: {res.stderr.strip()}"
        if exit_on_error:
            die(err_msg)
        else:
            print_error(err_msg)
    return res.returncode, res.stdout.strip()


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


def run_git(args: list[str], check: bool = True) -> tuple[int, str]:
    """Executes a Git command and returns its exit code and standard output.

    Args:
        args: The Git subcommand and its arguments.
        check: If True, exits the program if the Git command fails.

    Returns:
        The return code and the stripped standard output.

    Raises:
        SystemExit: If the Git command fails and `check` is True.
    """
    res = subprocess.run(
        ["git", *args],
        capture_output=True,
        text=True,
    )
    if check and res.returncode != 0:
        die(f"Git command failed: git {' '.join(args)}\nError: {res.stderr.strip()}")

    return res.returncode, res.stdout.strip()


def _resolve_branch_ref(branch: str) -> str | None:
    """Finds a usable ref for a branch, preferring the local branch over origin's.

    Args:
        branch: The branch name to look up.

    Returns:
        The local branch name, its `origin/` remote-tracking ref, or None if neither exists.
    """
    for ref in (branch, f"origin/{branch}"):
        code, _ = run_git(
            ["rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"], check=False
        )
        if code == 0:
            return ref
    return None


def resolve_base_branch(target: str | None = None) -> str:
    """Determines the target base ref to compare against.

    If no target is provided, queries the remote default branch or sequentially
    verifies the existence of 'main', 'master', and 'develop'. A branch that only
    exists on the remote (e.g. after `git clone -b feature`) resolves to its
    `origin/` ref so that it can be passed directly to Git.

    Args:
        target: A user-specified branch name, or None.

    Returns:
        A ref that Git can resolve: the local branch or its `origin/` counterpart.

    Raises:
        SystemExit: If no branch is provided and standard defaults are not found.
    """
    if target:
        ref = _resolve_branch_ref(target)
        if ref:
            return ref
        die(
            f"specified base branch '{target}' does not exist",
            hint="verify the branch name with 'git branch -a'",
        )

    # Dynamically query the default branch of the remote
    code, out = run_git(["symbolic-ref", "refs/remotes/origin/HEAD"], check=False)
    cleaned = out.strip()
    if code == 0 and cleaned:
        if cleaned.startswith("refs/remotes/origin/"):
            default_branch = cleaned.removeprefix("refs/remotes/origin/")
        else:
            default_branch = cleaned.split("/")[-1]
        ref = _resolve_branch_ref(default_branch)
        if ref:
            return ref

    fallbacks = ["main", "master", "develop"]
    for branch in fallbacks:
        ref = _resolve_branch_ref(branch)
        if ref:
            return ref

    die(
        "could not automatically detect a base branch (tried main, master, develop)",
        hint="specify it explicitly: focal wip-context <branch>",
    )
    return ""
