"""Shared fixtures for the migration suite.

`github` runs a local fake of the GitHub REST API. Tests register canned responses with
`github.add(...)`, then run the Rust binary with `github.env(...)`, which points the
client at the fake through FOCAL_GITHUB_API_URL and supplies a test token. Requests are
recorded on `github.requests` for assertions. `github_checkout` creates a Git repository
whose `origin` is a GitHub URL, so repository detection works without a network.
"""

import fcntl
import json
import os
import struct
import subprocess
import termios
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
OWNER, REPO = "octo", "widget"


@dataclass
class Reply:
    status: int = 200
    body: bytes = b""
    headers: dict[str, str] = field(default_factory=dict)


@dataclass
class Request:
    method: str
    path: str
    headers: dict[str, str]


class FakeGitHub:
    """Canned responses keyed by method and path; a full path with query wins over the
    bare path. A list of replies is served in order, repeating the last one."""

    def __init__(self) -> None:
        self.routes: dict[tuple[str, str], list[Reply]] = {}
        self.requests: list[Request] = []
        self.url = ""

    def add(
        self,
        path: str,
        body: object = None,
        *,
        status: int = 200,
        headers: dict[str, str] | None = None,
        method: str = "GET",
        raw: bytes | None = None,
    ) -> None:
        if raw is None:
            payload = b"" if body is None else json.dumps(body).encode()
            content_type = (
                {"Content-Type": "application/json"} if body is not None else {}
            )
        else:
            payload, content_type = raw, {}
        reply = Reply(status, payload, {**content_type, **(headers or {})})
        self.routes.setdefault((method, path), []).append(reply)

    def link(self, path: str) -> str:
        """An absolute URL on the fake, for Link headers and redirects."""
        return f"{self.url}{path}"

    def env(self, tmp_path: Path, **extra: str) -> dict[str, str]:
        home = tmp_path / "home"
        home.mkdir(exist_ok=True)
        path = "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"]
        return {
            "PATH": path,
            "HOME": str(home),
            "LC_ALL": "C.UTF-8",
            "NO_COLOR": "1",
            "GIT_CONFIG_NOSYSTEM": "1",
            "FOCAL_GITHUB_API_URL": self.url,
            "GH_TOKEN": "test-token",
            **extra,
        }

    def reply_for(self, method: str, target: str) -> Reply:
        parts = urlsplit(target)
        for key in ((method, target), (method, parts.path)):
            replies = self.routes.get(key)
            if replies:
                return replies.pop(0) if len(replies) > 1 else replies[0]
        return Reply(
            404, json.dumps({"message": f"no fixture for {method} {target}"}).encode()
        )


@pytest.fixture
def github():
    fake = FakeGitHub()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def handle_any(self):
            fake.requests.append(
                Request(self.command, self.path, dict(self.headers.items()))
            )
            reply = fake.reply_for(self.command, self.path)
            self.send_response(reply.status)
            for name, value in reply.headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(reply.body)))
            self.end_headers()
            self.wfile.write(reply.body)

        def do_GET(self):
            self.handle_any()

        def do_POST(self):
            self.handle_any()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    fake.url = f"http://127.0.0.1:{server.server_address[1]}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield fake
    server.shutdown()


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=a", "-c", "user.email=a@example.com", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1"},
    ).stdout.strip()


@pytest.fixture
def github_checkout(tmp_path):
    """A repository on branch `main` with one commit and origin set to octo/widget."""
    root = tmp_path / "checkout"
    root.mkdir()
    git(root, "init", "-q", "-b", "main")
    (root / "README.md").write_text("widget\n")
    git(root, "add", ".")
    git(root, "commit", "-qm", "initial")
    git(root, "remote", "add", "origin", f"https://github.com/{OWNER}/{REPO}.git")
    return root


def run_focal(cwd: Path, env: dict[str, str], *args: str, stdin: bytes | None = None):
    return subprocess.run(
        [CLI, *args],
        cwd=cwd,
        env=env,
        input=stdin,
        stdin=None if stdin is not None else subprocess.DEVNULL,
        capture_output=True,
        check=False,
    )


class Session:
    """Runs one command on a pseudo-terminal and records what it draws."""

    def __init__(self, cwd, *args, env=None):
        self.master, self.slave = os.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        self.name = os.ttyname(self.slave)
        self.before = termios.tcgetattr(self.slave)
        self.output = cwd.parent / "result.md"
        slave = self.slave

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

        path = "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"]
        with self.output.open("wb") as stdout:
            self.proc = subprocess.Popen(
                [CLI, *args],
                cwd=cwd,
                stdin=slave,
                stdout=stdout,
                stderr=subprocess.PIPE,
                preexec_fn=controlling_terminal,
                pass_fds=(slave,),
                env={
                    "PATH": path,
                    "HOME": str(cwd.parent),
                    "TERM": "xterm-256color",
                    "LC_ALL": "C.UTF-8",
                    "NO_COLOR": "1",
                    "FOCAL_OMIT_WORKSPACE": "1",
                    **(env or {}),
                },
            )
        self.screen = bytearray()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while True:
            try:
                chunk = os.read(self.master, 65536)
            except OSError:
                return
            if not chunk:
                return
            self.screen.extend(chunk)

    def wait_for(self, text, timeout=10.0):
        deadline = time.monotonic() + timeout
        while text.encode() not in self.screen:
            assert time.monotonic() < deadline, f"never drew {text!r}"
            time.sleep(0.02)

    def send(self, *keys, pause=0.3):
        for key in keys:
            os.write(self.master, key)
            time.sleep(pause)

    def finish(self):
        _, err = self.proc.communicate(timeout=20)
        try:
            after = termios.tcgetattr(self.slave)
        except termios.error:
            # macOS revokes the terminal from other holders when the session leader exits.
            fd = os.open(self.name, os.O_RDWR | os.O_NOCTTY)
            after = termios.tcgetattr(fd)
            os.close(fd)
        os.close(self.slave)
        os.close(self.master)
        return self.proc.returncode, err.decode(), self.output.read_text(), after


@pytest.fixture
def terminal():
    """Starts `focal <args>` on a pseudo-terminal: `terminal(cwd, *args, env=None)`.

    The process gets the terminal as its controlling TTY while stdout goes to
    `cwd.parent / "result.md"`. Use `wait_for`, `send`, and `finish` to drive it.
    """
    return Session


@pytest.fixture
def focal():
    """Runs the binary under test: `focal(cwd, env, *args, stdin=None)`."""
    return run_focal
