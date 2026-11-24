"""Interactive picker contracts for the Rust preview, driven through a pseudo-terminal.

Each run gives Focal a controlling terminal while stdout goes to a file, the way
`focal files > result.md` works. The legacy dispatcher uses fzf, which CI does not
install, so these run only against the Rust binary.
"""

import os
import subprocess
import termios
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()

pytestmark = pytest.mark.skipif(
    "FOCAL_TEST_BIN" not in os.environ, reason="the legacy picker is fzf"
)

ESC, TAB, ENTER, CTRL_A, CTRL_C, CTRL_U = (
    b"\x1b",
    b"\t",
    b"\r",
    b"\x01",
    b"\x03",
    b"\x15",
)


def assert_restored(session, after):
    assert after[3] == session.before[3], "terminal local modes were not restored"
    assert after[3] & termios.ICANON
    assert after[3] & termios.ECHO
    assert b"\x1b[?1049h" in session.screen
    assert session.screen.rindex(b"\x1b[?1049l") > session.screen.rindex(b"\x1b[?1049h")


@pytest.fixture
def repo(tmp_path):
    root = tmp_path / "repo"
    (root / "src").mkdir(parents=True)
    (root / "src/alpha.py").write_text("alpha = 1\n")
    (root / "src/beta.py").write_text("beta = 2\n")
    (root / "notes.md").write_text("notes\n")
    (root / "README.md").write_text("readme\n")
    return root


def headings(markdown):
    return [line for line in markdown.splitlines() if line.startswith("# ")]


def test_files_multi_select_survives_filtering(repo, terminal):
    session = terminal(repo, "files")
    session.wait_for("Files: ")
    session.wait_for("1 │ ")
    session.send(b"beta", TAB, CTRL_U, b"alpha", TAB, ENTER)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert headings(out) == ["# src/alpha.py", "# src/beta.py"]
    assert "added: src/alpha.py\nadded: src/beta.py\n" in err
    assert_restored(session, after)


def test_files_enter_takes_cursor_item_and_previews(repo, terminal):
    session = terminal(repo, "files")
    session.wait_for("TAB: multi-select | CTRL-A: select all | ENTER: confirm")
    session.send(b"notes")
    session.wait_for("1 │ notes")
    session.send(ENTER)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert headings(out) == ["# notes.md"]
    assert_restored(session, after)


def test_files_select_all(repo, terminal):
    session = terminal(repo, "files")
    session.wait_for("Files: ")
    session.send(b".py", CTRL_A, ENTER)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert headings(out) == ["# src/alpha.py", "# src/beta.py"]
    assert_restored(session, after)


@pytest.mark.parametrize("key", [ESC, CTRL_C])
def test_files_cancel(repo, terminal, key):
    session = terminal(repo, "files")
    session.wait_for("Files: ")
    session.send(TAB, key)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert out == ""
    assert err == "info: No files selected.\n"
    assert_restored(session, after)


def test_context_cancel_still_emits_core_context(repo, terminal):
    session = terminal(repo, "context", "notes.md")
    session.wait_for("Auto-included: README.md")
    session.wait_for("Pre-selected: notes.md")
    session.send(ESC)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert headings(out) == ["# Repository Tree", "# README.md", "# notes.md"]
    assert_restored(session, after)


def test_context_adds_picked_files_after_core_and_preselected(repo, terminal):
    session = terminal(repo, "context", "notes.md")
    session.wait_for("Additional files: ")
    # Core and pre-selected files are not offered again.
    session.send(b"beta", TAB, ENTER)
    code, err, out, after = session.finish()
    assert code == 0, err
    assert headings(out) == [
        "# Repository Tree",
        "# README.md",
        "# notes.md",
        "# src/beta.py",
    ]
    assert_restored(session, after)


def test_no_terminal(repo, terminal):
    result = subprocess.run(
        [CLI, "files"],
        cwd=repo,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
        env={"PATH": "", "HOME": str(repo.parent)},
        check=False,
    )
    assert result.returncode == 1
    assert b"interactive selection needs a terminal" in result.stderr
    assert b"pass paths or a flag such as --all" in result.stderr
