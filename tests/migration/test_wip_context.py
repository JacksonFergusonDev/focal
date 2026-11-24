"""`wip-context` contracts shared by the legacy dispatcher and the Rust preview.

Assertions check content, section order, and budgets rather than byte-identical Git
output: topology spacing, rename scores, and file-type descriptions may differ.
"""

import os
import re
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
    return subprocess.run(
        ["git", *args], cwd=cwd, env=env, check=True, capture_output=True, text=True
    ).stdout.strip()


def commit(cwd, env, message, when):
    dated = {**env, "GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when}
    git(cwd, dated, "add", "-A")
    git(cwd, dated, "commit", "-qm", message)


def run(cwd, env, *args):
    if os.environ.get("FOCAL_TEST_EMPTY_PATH"):
        env = {**env, "PATH": ""}
    return subprocess.run(
        [CLI, "wip-context", *args], cwd=cwd, env=env, capture_output=True, check=False
    )


@pytest.fixture
def repo(tmp_path, env):
    root = tmp_path / "repo"
    root.mkdir()
    git(root, env, "init", "-q", "-b", "main")
    (root / "README.md").write_text("readme\n")
    (root / "pkg").mkdir()
    (root / "pkg/core.py").write_text("value = 1\n")
    commit(root, env, "base", "2001-01-01T00:00:00Z")
    git(root, env, "checkout", "-qb", "feature")
    (root / "pkg/core.py").write_text("value = 2\n")
    (root / ".github").mkdir()
    (root / ".github/ci.yml").write_text("on: push\n")
    commit(root, env, "update core", "2001-01-02T00:00:00Z")
    (root / "README.md").write_text("readme\nmore\n")
    (root / "art.png").write_bytes(b"\x89PNG\r\n\x1a\n" + bytes(2000))
    commit(root, env, "docs and art", "2001-01-03T00:00:00Z")
    return root


def sections(payload):
    return re.findall(r"^#+ .*$", payload.decode(), flags=re.MULTILINE)


def test_branch_context(repo, env):
    (repo / "pkg/core.py").write_text("value = 3\n")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    base = git(repo, env, "merge-base", "main", "HEAD")[:7]
    head = git(repo, env, "rev-parse", "--short", "HEAD")
    assert out.startswith(
        f"<context>\n# WIP Branch Context\n**Base:** `main` ({base}) | **HEAD:** ({head})\n"
    )
    assert sections(result.stdout) == [
        "# WIP Branch Context",
        "## 0. Uncommitted Changes",
        "## 1. Branch Topology",
        "## 2. Macroscopic Map",
        "## 3. Microscopic Diffs",
        "## 4. Omitted Files Metadata",
        "### `.github/ci.yml`",
        "### `README.md`",
        "### `pkg/core.py`",
    ]
    assert "M pkg/core.py" in out.split("## 0. Uncommitted Changes")[1]
    topology = out.split("## 1. Branch Topology\n```text\n")[1].split("```")[0]
    newest, oldest = topology.index("docs and art ("), topology.index("update core (")
    assert newest < oldest
    assert re.search(r"\[[0-9a-f]{7}\] docs and art \(\d+ years ago\)", topology)
    assert "M\tpkg/core.py" in topology
    assert "A\t.github/ci.yml" in topology
    assert "A\tart.png" in topology
    stat = out.split("## 2. Macroscopic Map\n```text\n")[1].split("```")[0]
    assert "4 files changed" in stat
    assert "- `art.png` (Size: 2.0K)\n  - Type: " in out
    assert "+value = 2" in out
    assert "value = 3" not in out.split("## 3.")[1]
    assert "\nindex " not in out
    assert out.endswith("```\n</context>\n")
    assert result.stderr.startswith(b"done: Copied WIP branch context (~")


def test_clean_tree_omits_uncommitted_section(repo, env):
    result = run(repo, env, "main")
    assert result.returncode == 0, result.stderr
    assert b"## 0. Uncommitted Changes" not in result.stdout


def test_budget_truncates_and_keeps_fences_closed(repo, env):
    for n in range(5):
        (repo / "pkg" / f"big{n}.py").write_text(
            "".join(f"line {i} {'y' * 60}\n" for i in range(200))
        )
    commit(repo, env, "big files", "2001-01-04T00:00:00Z")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    diffs = result.stdout.decode().split("## 3. Microscopic Diffs")[1]
    assert "...[diff truncated: context limit reached]" in diffs
    match = re.search(r"diffs for (\d+) remaining files omitted", diffs)
    assert match
    assert int(match.group(1)) >= 2
    body = diffs.split("## 4. Omitted Files Metadata")[1]
    blocks = body.split("\n\n*...diffs for")[0]
    assert len(blocks) <= 30_500
    assert blocks.count("```") % 2 == 0


def test_remote_default_branch(repo, env):
    git(repo, env, "branch", "-m", "main", "trunk")
    git(repo, env, "update-ref", "refs/remotes/origin/trunk", "trunk")
    git(
        repo,
        env,
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/trunk",
    )
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    assert b"**Base:** `trunk` (" in result.stdout


def test_remote_only_fallback(repo, env):
    git(repo, env, "update-ref", "refs/remotes/origin/master", "main")
    git(repo, env, "branch", "-D", "main")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    assert b"**Base:** `origin/master` (" in result.stdout


def test_missing_base_branch(repo, env):
    result = run(repo, env, "nope")
    assert result.returncode == 1
    assert result.stdout == b""
    assert result.stderr == (
        b"error: specified base branch 'nope' does not exist\n"
        b"  hint: verify the branch name with 'git branch -a'\n"
    )


def test_no_detectable_base(repo, env):
    git(repo, env, "branch", "-D", "main")
    result = run(repo, env)
    assert result.returncode == 1
    assert b"could not automatically detect a base branch" in result.stderr


def test_no_divergence(repo, env):
    git(repo, env, "checkout", "-q", "main")
    result = run(repo, env)
    assert result.returncode == 1
    assert result.stdout == b""
    assert result.stderr == (
        b"error: no divergent commits found between 'main' and HEAD\n"
        b"  hint: create commits on this branch or specify a different base branch: "
        b"focal wip-context <branch>\n"
    )


def test_deleted_noise_file(repo, env):
    git(repo, env, "checkout", "-q", "main")
    (repo / "data.db").write_bytes(b"x")
    commit(repo, env, "db", "2001-01-01T12:00:00Z")
    git(repo, env, "checkout", "-qb", "drop")
    (repo / "data.db").unlink()
    (repo / "note.txt").write_text("n\n")
    commit(repo, env, "remove db", "2001-01-06T00:00:00Z")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    assert b"- `data.db` (Deleted or inaccessible)" in result.stdout


@pytest.mark.xfail(
    LEGACY, reason="legacy diffs a renamed path as a new file", strict=True
)
def test_rename_is_reported_as_rename(repo, env):
    (repo / "pkg/core.py").write_text("".join(f"line {n}\n" for n in range(20)))
    commit(repo, env, "grow core", "2001-01-06T00:00:00Z")
    git(repo, env, "checkout", "-q", "main")
    (repo / "pkg/core.py").write_text("".join(f"line {n}\n" for n in range(20)))
    commit(repo, env, "grow core on main", "2001-01-06T00:00:00Z")
    git(repo, env, "checkout", "-q", "feature")
    git(repo, env, "merge", "-q", "--no-edit", "main")
    git(repo, env, "mv", "pkg/core.py", "pkg/renamed.py")
    commit(repo, env, "rename", "2001-01-07T00:00:00Z")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert "rename from pkg/core.py\nrename to pkg/renamed.py" in out
