"""`web` contracts shared by the legacy dispatcher and the Rust preview.

A local HTTP server stands in for real sites. Assertions check content and structure,
since CI pins an older html-to-markdown CLI than local installs.
"""

import gzip
import os
import re
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()

PAGE = """<!doctype html><html><head><title>Fixture</title></head><body>
<header><a href="/">Site header link</a></header>
<nav><ul><li><a href="/a">Navigation item</a></li></ul></nav>
<main>
<h1>Main heading</h1>
<p>Body paragraph with <strong>bold</strong> text.</p>
<table><tr><th>Name</th><th>Value</th></tr><tr><td>alpha</td><td>1</td></tr></table>
<pre><code>print("code block")</code></pre>
<form><button>Submit button</button></form>
<svg><text>Vector label</text></svg>
</main>
<aside>Sidebar aside</aside>
<footer>Footer text</footer>
</body></html>"""

ERRORS = {
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    999: "Denied",
    500: "Oops",
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, status, body=b"", headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/page":
            self.send(200, PAGE.encode(), [("Content-Type", "text/html")])
        elif self.path == "/redirect":
            self.send(302, headers=[("Location", "/page")])
        elif self.path == "/gzip" and "gzip" in self.headers.get("Accept-Encoding", ""):
            body = gzip.compress(PAGE.encode())
            self.send(
                200, body, [("Content-Type", "text/html"), ("Content-Encoding", "gzip")]
            )
        elif self.path == "/long":
            long_page = "<p>" + "word " * 12_000 + "</p>"
            self.send(200, long_page.encode(), [("Content-Type", "text/html")])
        elif self.path == "/empty":
            self.send(200)
        elif self.path.lstrip("/").isdigit() and int(self.path[1:]) in ERRORS:
            self.send(int(self.path[1:]), ERRORS[int(self.path[1:])].encode())
        else:
            self.send(400, b"unexpected")


@pytest.fixture(scope="module")
def server():
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_address[1]}"
    httpd.shutdown()


@pytest.fixture
def env(tmp_path):
    return {
        "PATH": "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"],
        "HOME": str(tmp_path),
        "LC_ALL": "C.UTF-8",
        "NO_COLOR": "1",
    }


def run(env, *args, stdin=None):
    return subprocess.run(
        [CLI, "web", *args],
        env=env,
        input=stdin,
        stdin=None if stdin is not None else subprocess.DEVNULL,
        capture_output=True,
        check=False,
    )


def check_main_content(markdown):
    assert "Main heading" in markdown
    assert "**bold**" in markdown
    assert re.search(r"^\| Name +\| Value +\|$", markdown, re.MULTILINE)
    assert re.search(r"^\| alpha +\| 1 +\|$", markdown, re.MULTILINE)
    assert 'print("code block")' in markdown
    for excluded in (
        "Site header link",
        "Navigation item",
        "Submit button",
        "Vector label",
        "Sidebar aside",
        "Footer text",
    ):
        assert excluded not in markdown


@pytest.mark.parametrize("path", ["/page", "/redirect", "/gzip"])
def test_fetched_page(server, env, path):
    url = server + path
    result = run(env, url)
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert out.startswith(f"<context>\n# Source: {url}\n\n")
    assert "Workspace Context" not in out
    assert out.endswith("\n</context>\n")
    check_main_content(out)
    assert result.stderr.decode().startswith(
        f"done: Copied markdown context for {url} (~"
    )


def test_piped_html(env):
    result = run(env, stdin=PAGE.encode())
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert out.startswith("<context>\n# Source: Piped DOM/Clipboard\n\n")
    check_main_content(out)
    assert b"done: Copied markdown context from piped HTML (~" in result.stderr


def test_output_limit(server, env):
    result = run(env, server + "/long")
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert out.endswith(
        "\n\n...[web content truncated: exceeded 50000 characters]\n</context>\n"
    )
    body = out.split("\n\n", 1)[1].split("\n\n...[web content truncated")[0]
    assert len(body) == 50_000


@pytest.mark.parametrize(
    ("status", "message", "hint"),
    [
        (401, "HTTP 401 Unauthorized returned by 127.0.0.1", "requires authentication"),
        (403, "HTTP 403 Forbidden returned by 127.0.0.1", "pbpaste | focal web"),
        (404, "HTTP 404 Not Found returned for ", "URL path is spelled correctly"),
        (
            999,
            "HTTP 999 returned by 127.0.0.1 (anti-bot",
            "blocks automated CLI scrapers",
        ),
        (500, "HTTP 500 returned for ", "check network connectivity"),
    ],
)
def test_http_errors(server, env, status, message, hint):
    result = run(env, f"{server}/{status}")
    assert result.returncode == 1
    assert result.stdout == b""
    err = result.stderr.decode()
    assert f"error: {message}" in err
    hint_line = next(
        line for line in err.splitlines() if line.lstrip().startswith("hint:")
    )
    assert hint in hint_line


def test_empty_response(server, env):
    result = run(env, server + "/empty")
    assert result.returncode == 1
    assert b"empty response received" in result.stderr


def test_connection_failure(env):
    result = run(env, "http://127.0.0.1:9/")
    assert result.returncode == 1
    assert b"error: Error fetching http://127.0.0.1:9/: " in result.stderr
    assert b"hint: check network connectivity or URL validity" in result.stderr


def test_invalid_scheme(env):
    result = run(env, "ftp://example.com/file")
    assert result.returncode == 1
    assert b"error: Invalid URL scheme in 'ftp://example.com/file'" in result.stderr
    assert b"hint: URL must begin with http:// or https://" in result.stderr


def test_empty_stdin(env):
    result = run(env, stdin=b"  \n\t\n")
    assert result.returncode == 1
    assert b"error: received empty piped input" in result.stderr
