"""`search`, `symbol`, and `api` contracts shared by the legacy dispatcher and the Rust preview."""

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
        "FOCAL_OMIT_WORKSPACE": "1",
    }


def run(cwd, env, *args):
    if os.environ.get("FOCAL_TEST_EMPTY_PATH"):
        env = {**env, "PATH": ""}
    return subprocess.run(
        [CLI, *args], cwd=cwd, env=env, capture_output=True, check=False
    )


@pytest.fixture
def corpus(tmp_path, env):
    root = tmp_path / "corpus"
    (root / "pkg").mkdir(parents=True)
    (root / ".hidden").mkdir()
    subprocess.run(["git", "init", "-q"], cwd=root, env=env, check=True)
    (root / "lines.txt").write_text(
        "".join("hit\n" if n in (3, 10, 25) else f"l{n}\n" for n in range(1, 31))
    )
    (root / "pkg/mod.py").write_text(
        "class Widget:\n"
        "    def method(self):\n"
        "        return helper()\n"
        "async def _private():\n"
        "    pass\n"
        "def helper(x=-1):\n"
        "    return x\n"
        "# class NotAtColumnZero\n"
    )
    (root / "pkg/stub.pyi").write_text("def typed() -> int: ...\n")
    (root / "pkg/notes.md").write_text("def looks_like_python():\n")
    (root / "calls.js").write_text("helper();\n")
    (root / ".hidden/secret.py").write_text("def hidden():\n    hit\n")
    (root / ".gitignore").write_text("ignored.txt\n")
    (root / "ignored.txt").write_text("hit\n")
    (root / "binary.dat").write_bytes(b"hit\0data")
    (root / "tail.txt").write_bytes(b"no newline hit")
    return root


def records(payload):
    """Group ripgrep-style output by file, since file order is unspecified."""
    body = payload.decode().removeprefix("<context>\n").removesuffix("\n</context>\n")
    groups: dict[str, list[str]] = {}
    for line in body.split("\n"):
        if line == "--":
            continue
        path = line[: min(i for i in (line.find(":"), line.find("-", 2)) if i > 0)]
        groups.setdefault(path, []).append(line)
    return groups


def test_search_context(corpus, env):
    result = run(corpus, env, "search", "hit")
    assert result.returncode == 0, result.stderr
    payload = result.stdout.decode()
    found = records(result.stdout)
    assert sorted(found) == ["./lines.txt", "./tail.txt"]
    lines = [f"./lines.txt-l{n}" for n in range(1, 31)]
    for n in (3, 10, 25):
        lines[n - 1] = "./lines.txt:hit"
    # Matches 3 and 10 share one window; 25 starts a new group after a separator.
    assert found["./lines.txt"] == lines[:15] + lines[19:]
    assert "./lines.txt-l15\n--\n./lines.txt-l20\n" in payload
    assert found["./tail.txt"] == ["./tail.txt:no newline hit"]
    assert payload.count("--\n") == 2
    assert result.stderr.startswith(b"done: Search results copied to clipboard (~")


@pytest.mark.parametrize("args", [("--", "-1"), ("-1",)])
def test_search_leading_dash(corpus, env, args):
    result = run(corpus, env, "search", *args)
    assert result.returncode == 0, result.stderr
    assert "./pkg/mod.py:def helper(x=-1):" in records(result.stdout)["./pkg/mod.py"]


def test_search_no_matches(corpus, env):
    result = run(corpus, env, "search", "absent_term")
    assert result.returncode == 0
    assert result.stdout == b""
    assert result.stderr == b"info: No matches found for 'absent_term'.\n"


def test_search_missing_pattern(corpus, env):
    result = run(corpus, env, "search")
    assert result.returncode == 1
    assert result.stderr == (
        b"error: missing search pattern\n  hint: usage: focal search <pattern>\n"
    )


@pytest.mark.xfail(
    LEGACY, reason="legacy reports an invalid regex as no matches", strict=True
)
def test_search_invalid_regex(corpus, env):
    result = run(corpus, env, "search", "(")
    assert result.returncode == 1
    assert b"error: invalid regex '('" in result.stderr


def test_symbol_includes_call_sites(corpus, env):
    result = run(corpus, env, "symbol", "helper")
    assert result.returncode == 0, result.stderr
    payload = result.stdout.decode()
    blocks = payload.removeprefix("<context>\n# ").split("\n\n# ")
    assert sorted(block.split("\n")[0] for block in blocks) == [
        "./calls.js",
        "./pkg/mod.py",
    ]
    assert "# ./calls.js\n```js\nhelper();\n```" in payload
    assert payload.endswith("```\n</context>\n")
    assert result.stderr.startswith(
        b"done: Symbol context for 'helper' copied to clipboard (~"
    )


def test_symbol_no_matches(corpus, env):
    result = run(corpus, env, "symbol", "absent_term")
    assert result.returncode == 0
    assert result.stderr == b"info: No files found containing symbol 'absent_term'.\n"


def test_api_overview(corpus, env):
    result = run(corpus, env, "api")
    assert result.returncode == 0, result.stderr
    assert records(result.stdout) == {
        "./pkg/mod.py": [
            "./pkg/mod.py:class Widget:",
            "./pkg/mod.py:async def _private():",
            "./pkg/mod.py:def helper(x=-1):",
        ],
        "./pkg/stub.pyi": ["./pkg/stub.pyi:def typed() -> int: ..."],
    }
    assert result.stderr.startswith(b"done: API overview copied to clipboard (~")


def test_api_without_python(tmp_path, env):
    result = run(tmp_path, env, "api")
    assert result.returncode == 0
    assert result.stderr == b"info: No Python classes or functions found.\n"
