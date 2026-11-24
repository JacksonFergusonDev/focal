"""Public CLI contracts shared by the legacy dispatcher and the Rust preview."""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()


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
