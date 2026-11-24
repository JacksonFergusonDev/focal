"""Clipboard delivery contracts shared by the legacy dispatcher and the Rust preview.

Redirected output always goes to stdout. With a terminal on stdout, the payload goes to
the clipboard, or to stdout with a warning when no clipboard is available.
"""

import os
import pty
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ

linux = pytest.mark.skipif(
    sys.platform != "linux", reason="macOS copies in process, not via a helper"
)


def base_env(tmp_path, fake=None):
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    path = "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"]
    if fake:
        path = f"{fake}:{path}" if path else str(fake)
    return {
        "PATH": path,
        "HOME": str(home),
        "LC_ALL": "C",
        "NO_COLOR": "1",
        "FOCAL_OMIT_WORKSPACE": "1",
    }


def run_on_terminal(cwd, env, *args):
    """Run with a pseudo-terminal on stdout; return (terminal output, stderr, exit code)."""
    master, slave = pty.openpty()
    proc = subprocess.run(
        [CLI, *args],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=slave,
        stderr=subprocess.PIPE,
        check=False,
    )
    os.close(slave)
    shown = b""
    os.set_blocking(master, False)
    while True:
        try:
            chunk = os.read(master, 65536)
        except (BlockingIOError, OSError):
            break
        if not chunk:
            break
        shown += chunk
    os.close(master)
    return shown.replace(b"\r\n", b"\n"), proc.stderr.decode(), proc.returncode


@pytest.fixture
def fake_xclip(tmp_path):
    bin_dir = tmp_path / "fake-bin"
    bin_dir.mkdir()
    sink = tmp_path / "clipboard.txt"
    helper = bin_dir / "xclip"
    helper.write_text(f'#!/bin/sh\n/bin/cat > "{sink}"\n')
    helper.chmod(0o755)
    return bin_dir, sink


@linux
def test_terminal_copies_through_helper(tmp_path, fake_xclip):
    bin_dir, sink = fake_xclip
    (tmp_path / "note.txt").write_text("marker\n")
    env = {**base_env(tmp_path, bin_dir), "DISPLAY": ":99"}
    shown, err, code = run_on_terminal(tmp_path, env, "files", "note.txt")
    assert code == 0, err
    assert shown == b""
    copied = sink.read_bytes()
    assert copied.startswith(b"<context>\n# note.txt\n")
    assert copied.endswith(b"marker\n```\n\n\n</context>\n")
    assert "done: Copied note.txt to clipboard (~" in err


@linux
def test_redirected_output_skips_clipboard(tmp_path, fake_xclip):
    bin_dir, sink = fake_xclip
    (tmp_path / "note.txt").write_text("marker\n")
    env = {**base_env(tmp_path, bin_dir), "DISPLAY": ":99"}
    result = subprocess.run(
        [CLI, "files", "note.txt"],
        cwd=tmp_path,
        env=env,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert b"marker" in result.stdout
    assert not sink.exists()


@linux
@pytest.mark.skipif(
    LEGACY and any(shutil.which(n) for n in ("wl-copy", "xclip", "xsel")),
    reason="legacy uses any installed helper even without a display",
)
def test_terminal_without_clipboard_falls_back_to_stdout(tmp_path):
    (tmp_path / "note.txt").write_text("marker\n")
    shown, err, code = run_on_terminal(
        tmp_path, base_env(tmp_path), "files", "note.txt"
    )
    assert code == 0, err
    assert shown.startswith(b"<context>\n# note.txt\n")
    assert b"marker" in shown
    assert "Dumping to stdout..." in err


@pytest.mark.skipif(
    sys.platform != "darwin" or not os.environ.get("FOCAL_TEST_CLIPBOARD"),
    reason="replaces the real macOS clipboard; set FOCAL_TEST_CLIPBOARD=1 to run",
)
def test_macos_clipboard_persists_after_exit(tmp_path):
    saved = subprocess.run(["pbpaste"], capture_output=True, check=True).stdout
    try:
        (tmp_path / "note.txt").write_text(f"marker {os.getpid()}\n")
        shown, err, code = run_on_terminal(
            tmp_path, base_env(tmp_path), "files", "note.txt"
        )
        assert code == 0, err
        assert shown == b""
        pasted = subprocess.run(["pbpaste"], capture_output=True, check=True).stdout
        assert f"marker {os.getpid()}".encode() in pasted
        assert pasted.endswith(b"</context>\n")
    finally:
        subprocess.run(["pbcopy"], input=saved, check=True)
