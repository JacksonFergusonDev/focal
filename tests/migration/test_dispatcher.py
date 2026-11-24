"""Dispatcher contracts: help, the command list, global flags, and unknown commands."""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
COMMANDS = [
    "api",
    "ci-fail",
    "context",
    "diff",
    "files",
    "issues",
    "pr-diff",
    "release-context",
    "search",
    "symbol",
    "tree",
    "web",
    "wip-context",
]


def run(tmp_path, *args):
    path = "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"]
    return subprocess.run(
        [CLI, *args],
        cwd=tmp_path,
        env={"PATH": path, "HOME": str(tmp_path), "NO_COLOR": "1", "LC_ALL": "C"},
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=False,
    )


@pytest.mark.parametrize("args", [(), ("help",), ("-h",), ("--help",)])
def test_help_menu(tmp_path, args):
    result = run(tmp_path, *args)
    assert result.returncode == 0
    out = result.stdout.decode()
    assert out.startswith(
        "Usage: focal [global flags] <command> [args]\n\nGlobal flags:\n"
    )
    assert "  --repo <url>      Run the command on a remote GitHub repository" in out
    listed = [
        line.split()[0]
        for line in out.split("Available commands:\n")[1].splitlines()
        if line.startswith("  ") and not line.startswith("   ")
    ]
    assert listed == COMMANDS
    assert result.stderr == b""


def test_command_list_for_completion(tmp_path):
    result = run(tmp_path, "--commands")
    assert result.returncode == 0
    assert result.stdout.decode().splitlines() == COMMANDS


def test_unknown_command(tmp_path):
    result = run(tmp_path, "frobnicate")
    assert result.returncode == 1
    assert result.stdout == b""
    assert result.stderr == (
        b"error: unknown command 'frobnicate'\n"
        b"  hint: run 'focal help' for a list of available commands\n"
    )


@pytest.mark.parametrize("args", [("--repo",), ("tree", "--repo")])
def test_repo_flag_requires_value(tmp_path, args):
    result = run(tmp_path, *args)
    assert result.returncode == 1
    assert result.stderr.startswith(b"error: --repo requires an argument.\n  hint: ")


def test_double_dash_reaches_the_command(tmp_path):
    (tmp_path / "flags.txt").write_text("use --repo here\n")
    result = run(tmp_path, "search", "--", "--repo")
    assert result.returncode == 0, result.stderr
    assert b"./flags.txt:use --repo here" in result.stdout


def test_empty_repo_value_runs_locally(tmp_path):
    result = run(tmp_path, "--repo=", "tree")
    assert result.returncode == 0, result.stderr
    assert result.stdout.startswith(b"<context>\n")
