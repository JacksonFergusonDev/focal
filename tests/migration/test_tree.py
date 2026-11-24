"""Tree output through the public legacy and Rust entry points."""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ


@pytest.mark.parametrize("locale", ["C", "en_US.UTF-8"])
def test_tree_structure(tmp_path, locale):
    root = tmp_path / "fixture"
    (root / "d" / "empty").mkdir(parents=True)
    (root / "d" / "run").touch(mode=0o755)
    (root / "link").symlink_to("d")
    (root / ".gitignore").write_text("omit\n")
    (root / ".ignore").write_text("d\n")
    (root / "omit").touch()
    (root / "build").mkdir()
    (root / "z").touch()
    result = subprocess.run(
        [CLI, "tree"],
        cwd=root,
        capture_output=True,
        env={
            **os.environ,
            "LC_ALL": locale,
            "FOCAL_OMIT_WORKSPACE": "1",
            "PATH": ""
            if os.environ.get("FOCAL_TEST_EMPTY_PATH")
            else os.environ["PATH"],
        },
        check=True,
    )
    branch, end, bar = (
        ("|-- ", "`-- ", "|   ") if locale == "C" else ("├── ", "└── ", "│\xa0\xa0 ")
    )
    lines = result.stdout.decode().splitlines()
    assert lines[0] == "<context>"
    # tree versions differ in whether -F annotates the root and symlink target.
    assert lines[1] in {".", "./"}
    expected = [
        branch + ".gitignore",
        branch + ".ignore",
        branch + "d/",
        bar + branch + "empty/",
        bar + end + "run*",
        branch + "link -> d/",
        end + "z",
        "</context>",
    ]
    if LEGACY and locale != "C":
        # The external tree sorts siblings with the locale's collation, which ignores
        # leading dots under glibc but not on macOS; Rust always sorts by bytes.
        assert sorted(lines[2:]) == sorted(expected)
    else:
        assert lines[2:] == expected


def test_tree_depth(tmp_path):
    leaf = tmp_path / "1/2/3/4/5/6/7"
    leaf.mkdir(parents=True)
    result = subprocess.run(
        [CLI, "tree"],
        cwd=tmp_path,
        capture_output=True,
        check=True,
        env={
            **os.environ,
            "LC_ALL": "C",
            "FOCAL_OMIT_WORKSPACE": "1",
            "PATH": ""
            if os.environ.get("FOCAL_TEST_EMPTY_PATH")
            else os.environ["PATH"],
        },
    )
    assert b"6/" in result.stdout
    assert b"7/" not in result.stdout
