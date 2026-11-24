"""Non-interactive `context` contracts shared by the legacy dispatcher and the Rust preview."""

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
        "GIT_AUTHOR_NAME": "a",
        "GIT_AUTHOR_EMAIL": "a@example.com",
        "GIT_COMMITTER_NAME": "a",
        "GIT_COMMITTER_EMAIL": "a@example.com",
        "LC_ALL": "C",
        "NO_COLOR": "1",
        "FOCAL_OMIT_WORKSPACE": "1",
    }


def git(cwd, env, *args):
    subprocess.run(["git", *args], cwd=cwd, env=env, check=True, capture_output=True)


def run(cwd, env, *args):
    if os.environ.get("FOCAL_TEST_EMPTY_PATH"):
        env = {**env, "PATH": ""}
    return subprocess.run(
        [CLI, "context", *args], cwd=cwd, env=env, capture_output=True, check=False
    )


def sections(payload):
    """Split a payload into its top-level `# ` headings, in order."""
    return [line for line in payload.splitlines() if line.startswith("# ")]


@pytest.fixture
def repo(tmp_path, env):
    root = tmp_path / "repo"
    root.mkdir()
    git(root, env, "init", "-q", "-b", "main")
    (root / "Cargo.toml").write_text("[package]\n")
    (root / "README.md").write_text("Demo\n")
    (root / "big").write_text("one\ntwo\nthree\nfour\nfive\n")
    (root / "conf").write_text("base\n")
    (root / "del").write_text("d\n")
    (root / "mod").write_text("m\n")
    (root / "tc").write_text("t\n")
    git(root, env, "add", ".")
    git(root, env, "commit", "-qm", "init")
    git(root, env, "checkout", "-qb", "other")
    (root / "conf").write_text("theirs\n")
    git(root, env, "commit", "-qam", "theirs")
    git(root, env, "checkout", "-q", "main")
    (root / "conf").write_text("ours\n")
    git(root, env, "commit", "-qam", "ours")
    subprocess.run(
        ["git", "merge", "-q", "other"],
        cwd=root,
        env=env,
        check=False,
        capture_output=True,
    )
    git(root, env, "mv", "big", "renamed")
    with (root / "renamed").open("a") as handle:
        handle.write("six\n")
    git(root, env, "rm", "-q", "del")
    (root / "mod").write_text("mm\n")
    git(root, env, "add", "mod")
    (root / "mod").write_text("mmm\n")
    (root / "tc").unlink()
    (root / "tc").symlink_to("mod")
    (root / "a b").touch()
    (root / 'q"x').touch()
    (root / "é").touch()
    (root / "untracked/nested").mkdir(parents=True)
    (root / "untracked/nested/f").touch()
    (root / "ignored").mkdir()
    (root / "ignored/z.log").touch()
    (root / ".gitignore").write_text("*.log\n")
    return root


@pytest.mark.parametrize("subdir", [False, True])
def test_core_git_info(repo, env, subdir):
    cwd = repo
    if subdir:
        cwd = repo / "untracked"
    result = run(cwd, env, "--core")
    assert result.returncode == 0, result.stderr
    payload = result.stdout.decode()
    sha = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=repo,
        env=env,
        capture_output=True,
        check=True,
        text=True,
    ).stdout.strip()
    assert (
        "# Git Info\n```text\nBranch: main\nCommit: " + sha + "\n\nModified files:\n"
        "UU conf\nD  del\nMM mod\nRM big -> renamed\n T tc\n"
        '?? .gitignore\n?? "a b"\n?? "q\\"x"\n?? untracked/\n?? "\\303\\251"\n```\n\n'
    ) in payload
    expected = ["# Repository Tree", "# Git Info"]
    if not subdir:
        expected += ["# README.md", "# Cargo.toml"]
    assert sections(payload) == expected
    assert result.stderr.decode().startswith(
        "done: Context and files copied to clipboard (~"
    )


def test_core_outside_git_with_paths(tmp_path, env):
    (tmp_path / "README.md").write_text("readme\n")
    (tmp_path / "pyproject.toml").write_text("[project]\n")
    (tmp_path / "src").mkdir()
    (tmp_path / "src/b.py").write_text("b = 1\n")
    (tmp_path / "src/a.py").write_text("a = 1\n")
    (tmp_path / "extra.txt").write_text("extra\n")
    result = run(tmp_path, env, "-c", "extra.txt", "src/")
    assert result.returncode == 0, result.stderr
    payload = result.stdout.decode()
    assert payload.startswith("<context>\n# Repository Tree\n```text\n")
    assert "# Git Info" not in payload
    assert sections(payload)[1:] == [
        "# README.md",
        "# pyproject.toml",
        "# extra.txt",
        "# src/a.py",
        "# src/b.py",
    ]
    assert "# README.md\n```md\nreadme\n```\n\n" in payload
    assert payload.endswith("```\n\n\n</context>\n")


def test_core_empty_status(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "trunk")
    result = run(tmp_path, env, "--core")
    assert result.returncode == 0, result.stderr
    assert result.stdout.decode().endswith(
        "# Git Info\n```text\nBranch: trunk\nCommit: none\n\nModified files:\n```\n\n\n</context>\n"
    )


def test_missing_path(tmp_path, env):
    result = run(tmp_path, env, "--core", "absent.txt")
    assert result.returncode == 1
    assert b"file not found: absent.txt" in result.stderr
    assert result.stdout == b""


def test_all_includes_each_file_once(tmp_path, env):
    (tmp_path / "README.md").write_text("root\n")
    (tmp_path / ".hidden").write_text("h\n")
    (tmp_path / "lib").mkdir()
    (tmp_path / "lib/code.rs").write_text("fn main() {}\n")
    result = run(tmp_path, env, "--all", "lib/code.rs")
    assert result.returncode == 0, result.stderr
    found = sections(result.stdout.decode())
    assert found[:2] == ["# Repository Tree", "# README.md"]
    assert sorted(found[2:]) == ["# .hidden", "# lib/code.rs"]
    assert result.stderr.startswith(b"info: Discovering all valid files...\n")


@pytest.mark.xfail(
    LEGACY, reason="legacy fd --exclude drops nested manifests", strict=True
)
def test_all_keeps_nested_manifests(tmp_path, env):
    (tmp_path / "README.md").write_text("root\n")
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs/README.md").write_text("nested\n")
    result = run(tmp_path, env, "--all")
    assert result.returncode == 0, result.stderr
    assert sections(result.stdout.decode())[1:] == ["# README.md", "# docs/README.md"]


@pytest.mark.xfail(
    LEGACY, reason="legacy repeats manifests named explicitly", strict=True
)
def test_core_deduplicates_named_manifest(tmp_path, env):
    (tmp_path / "README.md").write_text("root\n")
    result = run(tmp_path, env, "--core", "README.md")
    assert result.returncode == 0, result.stderr
    assert sections(result.stdout.decode())[1:] == ["# README.md"]
