"""`diff` contracts shared by the legacy dispatcher and the Rust preview.

Assertions check content and section structure rather than byte-identical Git output:
stat column padding and rename similarity scores may differ between implementations.
"""

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
        [CLI, "diff", *args], cwd=cwd, env=env, capture_output=True, check=False
    )


def section(payload, name):
    """Return the body of a `### name` section, up to the next `### ` heading."""
    text = payload.decode()
    start = text.index(f"### {name}\n") + len(name) + 5
    end = text.find("\n### ", start)
    return text[start : end if end >= 0 else len(text)]


@pytest.fixture
def repo(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "main")
    (tmp_path / "main.py").write_text("initial\n")
    (tmp_path / "uv.lock").write_text("lock v1\n")
    (tmp_path / "staged.py").write_text("base\n")
    git(tmp_path, env, "add", ".")
    git(tmp_path, env, "commit", "-qm", "initial")
    return tmp_path


def test_unstaged_default_excludes_noise(repo, env):
    (repo / "main.py").write_text("initial\ncode change\n")
    (repo / "uv.lock").write_text("lock v2\n")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert out.startswith("<context>\n### Status\n```\n")
    assert " M main.py" in section(result.stdout, "Status")
    assert "main.py" in section(result.stdout, "Summary")
    assert "1 file changed, 1 insertion(+)" in section(result.stdout, "Summary")
    patch = section(result.stdout, "Diff")
    assert patch.startswith("```diff\ndiff --git a/main.py b/main.py\n")
    assert "+code change\n```\n" in patch
    assert "uv.lock" not in out
    assert "lock v2" not in out
    assert out.endswith("```\n\n\n</context>\n")
    assert result.stderr.startswith(b"done: Copied diff of working tree (~")


def test_only_noise_reports_no_changes(repo, env):
    (repo / "uv.lock").write_text("lock v2\n")
    result = run(repo, env)
    assert result.returncode == 0
    assert result.stdout == b""
    assert result.stderr == b"info: No changes detected in working tree.\n"


def test_explicit_path_includes_noise(repo, env):
    (repo / "uv.lock").write_text("lock v2\n")
    (repo / "main.py").write_text("changed\n")
    result = run(repo, env, "--", "uv.lock")
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert "+lock v2" in patch
    assert "main.py" not in result.stdout.decode()
    assert b"Copied diff of specified files" in result.stderr


def test_staged_warns_about_unstaged_and_untracked(repo, env):
    (repo / "staged.py").write_text("staged\n")
    git(repo, env, "add", "staged.py")
    (repo / "main.py").write_text("unstaged\n")
    (repo / "new.txt").write_text("new\n")
    result = run(repo, env, "--cached")
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert "+staged" in patch
    assert "unstaged" not in patch
    assert "new.txt" not in patch
    stderr = result.stderr.decode()
    assert "warning: You have unstaged changes not included in this diff." in stderr
    assert "warning: You have untracked files not included in this diff." in stderr
    assert "done: Copied diff of staged changes" in stderr


def test_unstaged_warns_about_staged(repo, env):
    (repo / "staged.py").write_text("staged\n")
    git(repo, env, "add", "staged.py")
    (repo / "main.py").write_text("unstaged\n")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    assert "+staged" not in section(result.stdout, "Diff")
    assert (
        b"warning: You have staged changes not included in this diff." in result.stderr
    )


def test_all_includes_untracked_and_deletions(repo, env):
    (repo / "staged.py").write_text("staged\n")
    git(repo, env, "add", "staged.py")
    (repo / "main.py").unlink()
    (repo / "with space.txt").write_text("spaced\n")
    (repo / "new.lock").write_text("noise\n")
    result = run(repo, env, "-a")
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert "+staged" in patch
    assert "-initial" in patch
    assert "deleted file mode" in patch
    assert "+++ b/with space.txt" in patch
    assert "+spaced" in patch
    assert "new.lock" not in result.stdout.decode()
    assert b"warning" not in result.stderr
    assert b"Copied diff of all changes" in result.stderr


def test_all_on_unborn_branch(tmp_path, env):
    git(tmp_path, env, "init", "-q", "-b", "main")
    (tmp_path / "first.py").write_text("one\n")
    git(tmp_path, env, "add", "first.py")
    (tmp_path / "loose.py").write_text("two\n")
    result = run(tmp_path, env, "--all")
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert "+one" in patch
    assert "+two" in patch


def test_fence_outgrows_backticks(repo, env):
    (repo / "main.py").write_text("initial\n````\n")
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert patch.startswith("`````diff\n")
    assert "+````\n`````\n" in patch


def test_large_diff_is_not_truncated(repo, env):
    body = "".join(f"line {n:06d} {'x' * 40}\n" for n in range(1000))
    (repo / "main.py").write_text(body)
    result = run(repo, env)
    assert result.returncode == 0, result.stderr
    patch = section(result.stdout, "Diff")
    assert len(patch) > 30_000
    assert "+line 000999" in patch


def test_thick_appends_tree_and_files(repo, env):
    (repo / "pkg").mkdir()
    (repo / "pkg/mod.py").write_text("x = 1\n")
    git(repo, env, "add", ".")
    git(repo, env, "commit", "-qm", "pkg")
    (repo / "pkg/mod.py").write_text("x = 2\n")
    result = run(repo, env, "--thick")
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    tree = section(result.stdout, "Macroscopic Map")
    assert tree.startswith("```text\n.")
    assert "mod.py" in tree
    grounding = section(result.stdout, "Contextual Grounding")
    assert grounding == "# pkg/mod.py\n```py\nx = 2\n```\n</context>\n"
    assert out.index("### Diff") < out.index("### Macroscopic Map")
    assert b"info: Generating thick diff contextual grounding..." in result.stderr


def test_outside_repository(tmp_path, env):
    result = run(tmp_path, env)
    assert result.returncode != 0
    assert result.stdout == b""
    assert b"not a git repository" in result.stderr
