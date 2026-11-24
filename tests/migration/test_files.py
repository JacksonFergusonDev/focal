"""Public CLI contracts shared by the legacy dispatcher and the Rust preview."""

import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ


@pytest.fixture
def env(tmp_path):
    home = tmp_path / "home"
    home.mkdir()
    return {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "GIT_CONFIG_NOSYSTEM": "1",
        "LC_ALL": "C",
        "NO_COLOR": "1",
    }


def run(cwd, env, *args):
    if os.environ.get("FOCAL_TEST_EMPTY_PATH"):
        env = {**env, "PATH": ""}
    return subprocess.run(
        [CLI, "files", *(args or ("--all",))],
        cwd=cwd,
        env=env,
        capture_output=True,
        check=False,
    )


@pytest.mark.parametrize("in_repo", [False, True])
def test_mixed_files(tmp_path, env, in_repo):
    root = tmp_path / "repo"
    root.mkdir()
    if in_repo:
        subprocess.run(
            ["git", "init", "-q", "-b", "main"], cwd=root, env=env, check=True
        )
    cwd = root / "src"
    cwd.mkdir()
    (cwd / "notes.md").write_bytes(b"keep\n```\ncode\n```\n\n")
    (cwd / ".empty").touch()
    (cwd / ".ignore").write_text("ignored.txt\n")
    (cwd / "ignored.txt").write_text("NEVER INCLUDE")
    result = run(cwd, env)
    assert result.returncode == 0, result.stderr
    payload = result.stdout.decode()
    header, body = payload.split("\n---\n\n")
    assert header.startswith("<context>\n> [!NOTE]\n")
    assert f"PWD: {cwd.resolve()}" in header
    if in_repo:
        assert header.endswith("Git Branch: main\nGit Commit: none\n")
    else:
        assert "Git Branch:" not in header
    blocks = [
        "# notes.md\n````md\nkeep\n```\ncode\n```\n````\n\n",
        "# .empty\n```empty\n[empty file]\n```\n\n",
        "# .ignore\n```ignore\nignored.txt\n```\n\n",
    ]
    for block in blocks:
        assert body.count(block) == 1
        body = body.replace(block, "")
    assert body == "\n</context>\n"
    assert (
        f"done: Copied context (~{len(payload) // 4} tokens)" in result.stderr.decode()
    )
    assert result.stderr.endswith(b"Added fully: 2 files\nAdded empty: 1 files\n")


@pytest.mark.parametrize(
    ("data", "marker"),
    [
        (b"x" * 100_001, "100000 of 100001 bytes"),
        (b"x\n" * 1_501, "1500 of 1501 lines"),
        (b"x" * 100_000, None),
    ],
)
def test_cutoffs(tmp_path, env, data, marker):
    cwd = tmp_path / "src"
    cwd.mkdir()
    (cwd / "long.txt").write_bytes(data)
    result = run(cwd, env)
    assert result.returncode == 0, result.stderr
    if marker:
        assert f"...[file truncated: showing first {marker}]".encode() in result.stdout
        assert b"Truncated (size limit): 1 files" in result.stderr
    else:
        assert b"...[file truncated" not in result.stdout
        assert data in result.stdout


def test_empty_directory(tmp_path, env):
    cwd = tmp_path / "empty"
    cwd.mkdir()
    result = run(cwd, env)
    assert result.returncode == 0
    assert result.stdout == b""
    assert b"No files selected." in result.stderr


def test_committed_repo_and_omissions(tmp_path, env):
    cwd = tmp_path / "repo"
    cwd.mkdir()
    subprocess.run(["git", "init", "-q", "-b", "main"], cwd=cwd, env=env, check=True)
    (cwd / ".gitignore").write_text("secret.txt\n")
    (cwd / "secret.txt").write_text("PRIVATE SENTINEL")
    subprocess.run(["git", "add", ".gitignore"], cwd=cwd, env=env, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture",
        ],
        cwd=cwd,
        env=env,
        check=True,
    )
    head = subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], cwd=cwd, env=env
    ).strip()
    (cwd / "bundle.MIN.JS").write_text("MINIFIED SENTINEL")
    (cwd / "unknown").write_bytes(b"\0\0\0\0\xff\xfe\0\0")
    result = run(cwd, env)
    assert result.returncode == 0, result.stderr
    assert b"Git Commit: " + head + b"\n" in result.stdout
    assert b"SENTINEL" not in result.stdout
    assert b"[asset/noise file omitted: bundle.MIN.JS]" in result.stdout
    assert b"[binary file omitted: unknown]" in result.stdout
    assert b"Skipped (noise/binary): 2 files" in result.stderr
    assert b"done: Copied .gitignore to clipboard" in result.stderr


def test_direct_selection(tmp_path, env):
    root = tmp_path / "src"
    root.mkdir()
    (root / "a.txt").write_text("A")
    (root / "b.txt").write_text("B")
    (root / ".fdignore").write_text("b.txt\n")
    result = run(tmp_path, env, "src/b.txt", "src/", "src/a.txt")
    assert result.returncode == 0, result.stderr
    assert result.stdout.count(b"# src/b.txt\n") == 1
    assert result.stdout.count(b"# src/a.txt\n") == 1
    assert result.stdout.index(b"# src/b.txt\n") < result.stdout.index(b"# src/a.txt\n")
    result = run(tmp_path, env, "src")
    assert result.returncode != 0
    assert b"trailing slash" in result.stderr
    assert not result.stdout


def test_walk_uses_fd_ignore_rules(tmp_path, env):
    cwd = tmp_path / "repo"
    cwd.mkdir()
    config = tmp_path / "config"
    env["XDG_CONFIG_HOME"] = str(config)
    (config / "fd").mkdir(parents=True)
    (config / "fd" / "ignore").write_text("global.txt\n")
    (cwd / ".fdignore").write_text("local.txt\n")
    (cwd / ".git").mkdir()
    for filename in [".git/private", "local.txt", "global.txt"]:
        (cwd / filename).write_text("EXCLUDED")
    (cwd / "visible").write_text("included")
    result = run(cwd, env)
    assert result.returncode == 0, result.stderr
    assert b"EXCLUDED" not in result.stdout
    assert b"# visible\n" in result.stdout


def test_detached_head(tmp_path, env):
    subprocess.run(
        ["git", "init", "-q", "-b", "main"], cwd=tmp_path, env=env, check=True
    )
    args = ["git", "-c", "user.name=Test", "-c", "user.email=test@example.com"]
    subprocess.run(
        [*args, "commit", "-qm", "root", "--allow-empty"],
        cwd=tmp_path,
        env=env,
        check=True,
    )
    subprocess.run(
        ["git", "checkout", "-q", "--detach"], cwd=tmp_path, env=env, check=True
    )
    (tmp_path / "sample").write_text("hello")
    sha = subprocess.check_output(
        ["git", "rev-parse", "--short", "HEAD"], cwd=tmp_path, env=env
    ).strip()
    result = run(tmp_path, env)
    assert result.returncode == 0, result.stderr
    assert b"Git Branch: \nGit Commit: " + sha + b"\n" in result.stdout


def test_notebook_rendering(tmp_path, env):
    path = tmp_path / "demo.IPYNB"
    payload = {
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {"kernelspec": {"language": "Julia", "display_name": "Julia 1"}},
        "cells": [
            {"cell_type": "markdown", "source": ["Example\n", "```"]},
            {
                "cell_type": "code",
                "source": "answer()",
                "execution_count": 0,
                "outputs": [
                    {
                        "output_type": "stream",
                        "name": "stderr",
                        "text": ["\x1b[31mé\x1b[0m", "é" * 4000],
                    },
                    {
                        "output_type": "display_data",
                        "data": {"text/plain": "fallback", "image/png": "unused"},
                    },
                    {
                        "output_type": "execute_result",
                        "data": {"text/html": "<p>html</p>", "text/latex": "unused"},
                    },
                    {
                        "output_type": "display_data",
                        "data": {"application/json": {"z": "é", "a": True}},
                    },
                    {
                        "output_type": "error",
                        "ename": "E",
                        "evalue": "failed",
                        "traceback": ["\x1b[32mline\x1b[0m"],
                    },
                ],
            },
            {"cell_type": "code", "source": " \n"},
            {"cell_type": "raw", "metadata": {"format": "latex"}, "source": "raw"},
        ],
    }
    path.write_text(json.dumps(payload))
    result = run(tmp_path, env, path.name)
    assert result.returncode == 0, result.stderr
    document = result.stdout.decode().split("\n---\n\n")[1]
    assert document == (
        "# Notebook: demo.IPYNB\n\n## Notebook Metadata\n\n"
        "- Kernel: Julia 1\n- Language: julia\n- Format: nbformat 4.5\n"
        "- Cells: 4 total (2 code, 1 markdown, 1 raw)\n\n"
        "## Markdown cell 1\n\n````markdown\nExample\n```\n````\n\n"
        "## Code cell 2 [execution: 0]\n\n```julia\nanswer()\n```\n\n"
        "### Output\n\n```text\n[stderr]\n"
        + "é"
        * 4000
        + "\n...[output truncated]\n```\n\n### Output\n\n[image output omitted]\n\n"
        "### Output\n\n```text\nhtml\n```\n\n"
        '### Output\n\n```json\n{\n  "z": "\\u00e9",\n  "a": true\n}\n```\n\n'
        "### Output\n\n```text\n[error]\nE: failed\nline\n```\n\n"
        "## Raw cell 4\n\n```latex\nraw\n```\n\n\n</context>\n"
    )


@pytest.mark.parametrize("source", ["", "[]", "{"])
def test_empty_and_malformed_notebooks(tmp_path, env, source):
    (tmp_path / "invalid.ipynb").write_text(source)
    (tmp_path / "keep.txt").write_text("still here")
    result = run(tmp_path, env, "invalid.ipynb", "keep.txt")
    assert result.returncode == 0, result.stderr
    assert b"# keep.txt\n" in result.stdout
    if source:
        assert b"# Notebook: invalid.ipynb\n\n[Error parsing notebook:" in result.stdout
    else:
        assert b"\n[empty file]\n\n" in result.stdout


def git(cwd, env, *args):
    subprocess.run(
        ["git", "-c", "user.name=a", "-c", "user.email=a@example.com", *args],
        cwd=cwd,
        env=env,
        check=True,
        capture_output=True,
    )


@pytest.fixture
def changes(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "main")
    (tmp_path / "sub").mkdir()
    for name in ("gone", "moved", "edited", "sub/inner", "sub/clean"):
        (tmp_path / name).write_text(f"{name}\n")
    git(tmp_path, env, "add", ".")
    git(tmp_path, env, "commit", "-qm", "init")
    git(tmp_path, env, "rm", "-q", "gone")
    git(tmp_path, env, "mv", "moved", "renamed")
    (tmp_path / "edited").write_text("staged\n")
    (tmp_path / "new").write_text("new\n")
    git(tmp_path, env, "add", "edited", "new")
    (tmp_path / "edited").write_text("staged\nthen edited\n")
    (tmp_path / "sub/inner").write_text("unstaged\n")
    (tmp_path / "untracked").write_text("u\n")
    return tmp_path


def headings(result):
    return [
        line
        for line in result.stdout.decode().splitlines()
        if line.startswith("# ") and line != "# Workspace Context"
    ]


def test_staged_selection(changes, env):
    result = run(changes, env, "--staged")
    assert result.returncode == 0, result.stderr
    assert headings(result) == ["# edited", "# new", "# renamed"]
    assert b"# edited\n```\nstaged\nthen edited\n```\n" in result.stdout
    assert result.stderr.startswith(b"info: Discovering staged files...\n")


def test_staged_unborn_branch(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "main")
    (tmp_path / "first").write_text("1\n")
    git(tmp_path, env, "add", "first")
    result = run(tmp_path, env, "-s")
    assert result.returncode == 0, result.stderr
    assert headings(result) == ["# first"]


@pytest.mark.parametrize("subdir", [False, True])
def test_modified_selection(changes, env, subdir):
    cwd = changes / "sub" if subdir else changes
    result = run(cwd, env, "--modified")
    assert result.returncode == 0, result.stderr
    expected = ["# inner"] if subdir else ["# edited", "# sub/inner"]
    assert headings(result) == expected
    assert result.stderr.startswith(b"info: Discovering modified files...\n")


@pytest.mark.xfail(
    LEGACY, reason="legacy reads root-relative staged paths", strict=True
)
def test_staged_from_subdirectory(changes, env):
    result = run(changes / "sub", env, "--staged")
    assert result.returncode == 0, result.stderr
    assert headings(result) == ["# ../edited", "# ../new", "# ../renamed"]


@pytest.mark.xfail(LEGACY, reason="legacy lists each conflict stage", strict=True)
def test_modified_conflict_listed_once(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "main")
    (tmp_path / "conf").write_text("base\n")
    git(tmp_path, env, "add", ".")
    git(tmp_path, env, "commit", "-qm", "base")
    git(tmp_path, env, "checkout", "-qb", "other")
    (tmp_path / "conf").write_text("theirs\n")
    git(tmp_path, env, "commit", "-qam", "theirs")
    git(tmp_path, env, "checkout", "-q", "main")
    (tmp_path / "conf").write_text("ours\n")
    git(tmp_path, env, "commit", "-qam", "ours")
    merge = subprocess.run(
        [
            "git",
            "-c",
            "user.name=a",
            "-c",
            "user.email=a@example.com",
            "merge",
            "other",
        ],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        check=False,
    )
    assert b"CONFLICT" in merge.stdout + merge.stderr
    result = run(tmp_path, env, "-m")
    assert result.returncode == 0, result.stderr
    assert headings(result) == ["# conf"]
