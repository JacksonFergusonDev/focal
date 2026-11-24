"""`release-context` GitHub enrichment, against a local fake of the REST and GraphQL APIs.

The legacy command reaches GitHub through `gh`, so these tests run only against the
Rust binary. The fake answers GraphQL association queries per commit from the request
body, serves REST routes in order, and can delay a route to exercise the per-request
timeout and the shared collection deadline. Nothing here contacts real GitHub.
"""

import json
import os
import re
import subprocess
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
EPOCH = datetime(2001, 1, 1, tzinfo=timezone.utc)
SLUG = "octo/widget"

pytestmark = pytest.mark.skipif(
    "FOCAL_TEST_BIN" not in os.environ,
    reason="legacy release-context reaches GitHub through the gh CLI",
)


@dataclass
class Reply:
    status: int = 200
    body: Any = None
    headers: dict[str, str] = field(default_factory=dict)
    delay: float = 0


class FakeAPI:
    """REST routes keyed by path (a full path with query wins), plus a GraphQL endpoint
    that answers `object(oid: ...)` aliases from `associations`."""

    def __init__(self) -> None:
        self.routes: dict[str, list[Reply]] = {}
        self.associations: dict[str, list[dict[str, Any]]] = {}
        self.overflow: set[str] = set()
        self.graphql: Reply | None = None
        self.graphql_delay: dict[str, float] = {}
        self.requests: list[tuple[str, str, dict[str, str]]] = []
        self.url = ""
        # Served when a test registers nothing for the path.
        self.defaults = {
            f"/repos/{SLUG}": Reply(
                body={"full_name": SLUG, "html_url": f"https://github.com/{SLUG}"}
            ),
            f"/repos/{SLUG}/releases": Reply(body=[]),
        }

    def add(self, path: str, body: Any = None, **kwargs: Any) -> None:
        self.routes.setdefault(path, []).append(Reply(body=body, **kwargs))

    def link(self, path: str) -> dict[str, str]:
        return {"Link": f'<{self.url}{path}>; rel="next"'}

    def rest(self, target: str) -> Reply:
        for key in (target, urlsplit(target).path):
            replies = self.routes.get(key)
            if replies:
                return replies.pop(0) if len(replies) > 1 else replies[0]
        return self.defaults.get(
            urlsplit(target).path, Reply(404, {"message": "Not Found"})
        )

    def answer_graphql(self, body: bytes) -> Reply:
        if self.graphql is not None:
            return self.graphql
        query = json.loads(body)["query"]
        aliases = re.findall(r'(c\d+): object\(oid: "([0-9a-f]+)"\)', query)
        data: dict[str, Any] = {}
        delay = 0.0
        for alias, sha in aliases:
            delay = max(delay, self.graphql_delay.get(sha, 0))
            nodes = [
                {
                    "number": pr["number"],
                    "title": pr["title"],
                    "body": pr["body"],
                    "url": pr["html_url"],
                    "mergedAt": pr["merged_at"],
                    "mergeCommit": {"oid": pr["merge_commit_sha"]}
                    if pr["merge_commit_sha"]
                    else None,
                    "author": {
                        "login": pr["user"]["login"],
                        "__typename": pr["user"]["type"],
                    },
                    "labels": {
                        "nodes": pr["labels"],
                        "pageInfo": {"hasNextPage": False},
                    },
                }
                for pr in self.associations.get(sha, [])
            ]
            data[alias] = {
                "associatedPullRequests": {
                    "pageInfo": {"hasNextPage": sha in self.overflow},
                    "nodes": nodes,
                }
            }
        payload = {
            "data": {
                "repository": data,
                "rateLimit": {"remaining": 4000, "resetAt": "2001-01-01T01:00:00Z"},
            }
        }
        return Reply(body=payload, delay=delay)

    def paths(self, fragment: str) -> list[str]:
        return [path for _, path, _ in self.requests if fragment in path]


@pytest.fixture
def api():
    fake = FakeAPI()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def respond(self, body: bytes = b""):
            headers = {name.lower(): value for name, value in self.headers.items()}
            fake.requests.append((self.command, self.path, headers))
            if self.command == "POST":
                reply = fake.answer_graphql(body)
            else:
                reply = fake.rest(self.path)
            time.sleep(reply.delay)
            payload = json.dumps(reply.body).encode()
            self.send_response(reply.status)
            self.send_header("Content-Type", "application/json")
            for name, value in reply.headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def do_GET(self):
            self.respond()

        def do_POST(self):
            self.respond(self.rfile.read(int(self.headers["Content-Length"])))

    class Server(ThreadingHTTPServer):
        daemon_threads = True

        def handle_error(self, request, client_address):
            # Clients that time out close the socket mid-response.
            pass

    server = Server(("127.0.0.1", 0), Handler)
    fake.url = f"http://127.0.0.1:{server.server_address[1]}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield fake
    server.shutdown()


@pytest.fixture
def env(tmp_path, api):
    home = tmp_path / "home"
    home.mkdir()
    empty = tmp_path / "bin"
    empty.mkdir()
    return {
        # No gh: credentials come from GH_TOKEN alone.
        "PATH": str(empty),
        "HOME": str(home),
        "LC_ALL": "C",
        "NO_COLOR": "1",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GH_TOKEN": "test-token",
        "FOCAL_GITHUB_API_URL": api.url,
    }


class Repo:
    """A Git repository with an octo/widget origin and fixed, distinct commit dates."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.tick = 0
        root.mkdir(parents=True)
        self.git("init", "-q", "-b", "main")
        self.git("remote", "add", "origin", f"https://github.com/{SLUG}.git")

    def git(self, *args: str, author: str = "Alice <alice@example.com>") -> str:
        self.tick += 1
        when = (EPOCH + timedelta(minutes=self.tick)).isoformat()
        name, email = author[:-1].split(" <")
        env = {
            **os.environ,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": name,
            "GIT_AUTHOR_EMAIL": email,
            "GIT_COMMITTER_NAME": "Alice",
            "GIT_COMMITTER_EMAIL": "alice@example.com",
            "GIT_AUTHOR_DATE": when,
            "GIT_COMMITTER_DATE": when,
        }
        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            env=env,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def commit(self, subject: str, path: str = "app.txt", **kwargs: str) -> str:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("a") as file:
            file.write(subject + "\n")
        self.git("add", "-A")
        self.git("commit", "-qm", subject, **kwargs)
        return self.git("rev-parse", "HEAD")


@pytest.fixture
def repo(tmp_path):
    return Repo(tmp_path / "repo")


def pr(number, *, author="alice", merged=True, merge_commit=None, bot=False, **kwargs):
    return {
        "number": number,
        "title": f"Change {number}",
        "body": f"Intent for #{number}\n<!-- hidden template -->",
        "html_url": f"https://github.com/{SLUG}/pull/{number}",
        "merged_at": "2001-02-01T00:00:00Z" if merged else None,
        "merge_commit_sha": merge_commit,
        "user": {"login": author, "type": "Bot" if bot else "User"},
        "labels": [{"name": "enhancement"}],
        **kwargs,
    }


def release(tag, **kwargs):
    return {"tag_name": tag, "draft": False, "prerelease": False, **kwargs}


def view(tag, **kwargs):
    return {
        "tag_name": tag,
        "name": f"Title of {tag}",
        "body": f"## Highlights of {tag}",
        "html_url": f"https://github.com/{SLUG}/releases/tag/{tag}",
        **kwargs,
    }


def run(repo, env, *args):
    started = time.monotonic()
    result = subprocess.run(
        [CLI, "release-context", *args],
        cwd=repo.root,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        check=False,
    )
    elapsed = time.monotonic() - started
    assert result.returncode == 0, result.stderr.decode()
    return result.stdout.decode(), result.stderr.decode(), elapsed


def section(out, heading):
    return out.split(f"\n## {heading}")[1].split("\n## ")[0]


def test_enriched_report_groups_commits_and_pins_assets(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    first = repo.commit("Fix one", "one.txt")
    second = repo.commit("Fix two", "docs/demo.png")
    direct = repo.commit("Direct push", "direct.txt")
    bot = repo.commit(
        "Bump lib", "deps.txt", author="dependabot[bot] <bot@example.com>"
    )
    head = bot
    api.associations = {
        first: [pr(1)],
        second: [pr(1)],
        bot: [pr(3, author="dependabot", bot=True, body="Verbose bot notes")],
    }
    api.add(f"/repos/{SLUG}/releases", [release("v1.0.0"), release("v0.9.0")])
    api.add(f"/repos/{SLUG}/releases/tags/v1.0.0", view("v1.0.0"))
    out, _, _ = run(repo, env)

    index = section(out, "PR Index (all associated PRs)")
    assert (
        f"* [#1: Change 1](https://github.com/{SLUG}/pull/1) — 2 in-range commits"
        in index
    )
    assert "#3: Change 3" in index
    prs = section(out, "Pull Requests")
    assert prs.count("### PR #1: Change 1") == 1
    assert "**Author:** @alice" in prs
    assert "**Labels:** enhancement" in prs
    assert "Intent for #1" in prs
    assert "hidden template" not in out
    assert f"* `{first[:12]}` — Fix one (Alice)" in prs
    assert "* `docs/demo.png`" in prs
    assert "* `one.txt`" in prs
    assert "Verbose bot notes" not in out
    assert "*All associated PRs" not in prs
    unmatched = section(out, "Unmatched Commits")
    assert f"* `{direct[:12]}` — Direct push (Alice)" in unmatched
    assert first[:12] not in unmatched
    bots = section(out, "Bot Updates (compact summary)")
    assert (
        f"* PR #3: Change 3 — https://github.com/{SLUG}/pull/3; 1 commits, 1 paths"
        in bots
    )

    reference = section(out, "Previous Release (style reference)")
    assert "**Tag:** v1.0.0\n**Title:** Title of v1.0.0" in reference
    assert "## Highlights of v1.0.0" in out
    assets = section(out, "Repository Assets (pinned to release commit)")
    assert (
        f"* `docs/demo.png` — [file](https://github.com/{SLUG}/blob/{head}/docs/demo.png), "
        f"[raw](https://github.com/{SLUG}/raw/{head}/docs/demo.png)"
    ) in assets

    notes = section(out, "Completeness Notes")
    assert (
        "Collected 4 commits; 2 merged PRs; 1 unmatched commits; 0 unresolved association lookups; 0 ambiguous associations."
        in notes
    )
    assert "Bot policy: summary; 1 bot PRs and 1 commits summarized." in notes
    assert "1 published release tags were not reachable in the local checkout" in notes
    assert "nearest locally reachable published stable release" in notes
    assert (
        "GitHub collection made 4 requests; per-request timeout 10s, total GitHub budget 60s."
        in notes
    )
    assert "WARNING" not in notes
    assert all(
        headers.get("authorization") == "Bearer test-token"
        for _, _, headers in api.requests
    )
    assert [method for method, _, _ in api.requests].count("POST") == 1


def test_ambiguous_association_prefers_the_merge_commit(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    sha = repo.commit("Shared fix")
    api.associations = {sha: [pr(1), pr(2, merge_commit=sha), pr(3, merged=False)]}
    out, _, _ = run(repo, env)
    assert "### PR #2: Change 2" in out
    assert "### PR #1:" not in out
    assert "#3" not in section(out, "PR Index (all associated PRs)")
    notes = section(out, "Completeness Notes")
    assert (
        f"Commit `{sha[:12]}` has multiple merged PR associations: #1, #2; grouped under #2."
        in notes
    )
    assert (
        "1 merged PRs; 0 unmatched commits; 0 unresolved association lookups; 1 ambiguous associations."
        in notes
    )


def test_association_pages_are_followed_and_page_two_failures_stay_unresolved(
    repo, env, api
):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    complete = repo.commit("Popular commit", "a.txt")
    broken = repo.commit("Broken listing", "b.txt")
    api.overflow = {complete, broken}
    for sha, second in ((complete, None), (broken, 422)):
        base = f"/repos/{SLUG}/commits/{sha}/pulls?per_page=100"
        api.add(
            base,
            [pr(5, merged=False), pr(7 if second else 6, merged=False)],
            headers=api.link(base + "&page=2"),
        )
        if second:
            api.add(base + "&page=2", {"message": "Validation Failed"}, status=second)
        else:
            api.add(base + "&page=2", [pr(6)])
    out, _, _ = run(repo, env)
    assert "### PR #6: Change 6" in out
    assert f"* `{complete[:12]}` — Popular commit (Alice)" in section(
        out, "Pull Requests"
    )
    assert f"* `{broken[:12]}` — Broken listing (Alice)" in section(
        out, "Unmatched Commits"
    )
    notes = section(out, "Completeness Notes")
    assert (
        "1 merged PRs; 1 unmatched commits; 1 unresolved association lookups" in notes
    )
    assert f"WARNING: PR enrichment is incomplete for: `{broken[:12]}`" in notes
    assert "Unresolved association lookups: 1 failed, 0 skipped" in notes
    assert (
        f"WARNING: GitHub request failed: GET repos/{SLUG}/commits/{broken}/pulls?per_page=100&page=2: HTTP 422: Validation Failed"
        in notes
    )
    assert len(api.paths("&page=2")) == 2


def test_deadline_expiry_midway_keeps_completed_results(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    quick = repo.commit("Quick", "quick.txt")
    slow = [repo.commit(f"Slow {i}", f"slow{i}.txt") for i in range(10)]
    api.associations = {quick: [pr(1)]}
    api.overflow = set(slow)
    for number, sha in enumerate(slow, 10):
        api.add(
            f"/repos/{SLUG}/commits/{sha}/pulls?per_page=100", [pr(number)], delay=1.2
        )
    out, err, elapsed = run(
        repo, env, "--request-timeout", "5", "--github-timeout", "2"
    )
    # Four listings finish in the first wave; the next four are capped by the remaining
    # budget and time out; the last two are never requested.
    assert elapsed < 6
    prs = section(out, "Pull Requests")
    assert "### PR #1: Change 1" in prs
    for number in range(10, 14):
        assert f"### PR #{number}:" in prs
    for number in range(14, 20):
        assert f"### PR #{number}:" not in prs
    notes = section(out, "Completeness Notes")
    assert "6 unresolved association lookups" in notes
    assert "Unresolved association lookups: 4 failed, 2 skipped" in notes
    assert re.search(
        r"WARNING: GET repos/\S+ timed out after 0\.\ds; remaining GitHub enrichment skipped\.",
        notes,
    )
    assert "total GitHub budget 2s" in notes
    assert len(api.paths("/pulls?per_page=100")) == 8
    assert "timed out" in err


def test_rate_limit_stops_scheduling(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    shas = [repo.commit(f"Change {i}", f"f{i}.txt") for i in range(3)]
    api.graphql = Reply(
        403,
        {"message": "API rate limit exceeded for user."},
        {"x-ratelimit-remaining": "0", "x-ratelimit-reset": "978310000"},
    )
    out, _, _ = run(repo, env)
    notes = section(out, "Completeness Notes")
    assert (
        "WARNING: GitHub rate limit reached; rerun after the limit resets (no automatic retries); "
        "remaining GitHub enrichment skipped. Local Git evidence is preserved."
    ) in notes
    assert "3 unresolved association lookups" in notes
    assert "Unresolved association lookups: 0 failed, 3 skipped" in notes
    assert api.paths("/pulls") == []
    for sha in shas:
        assert sha[:12] in section(out, "Unmatched Commits")


def test_graphql_errors_fall_back_to_rest_listings(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    sha = repo.commit("Fix")
    api.graphql = Reply(
        200, {"data": None, "errors": [{"message": "Something went wrong"}]}
    )
    api.add(f"/repos/{SLUG}/commits/{sha}/pulls?per_page=100", [pr(4)])
    out, _, _ = run(repo, env)
    assert "### PR #4: Change 4" in out
    notes = section(out, "Completeness Notes")
    assert "WARNING: GitHub request failed: POST graphql:" in notes
    assert "Something went wrong" in notes
    assert "0 unresolved association lookups" in notes


def test_missing_credentials_keep_the_local_report(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    repo.commit("Change")
    del env["GH_TOKEN"]
    out, _, _ = run(repo, env)
    notes = section(out, "Completeness Notes")
    assert (
        "WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment."
        in notes
    )
    assert (
        "GitHub enrichment unavailable: no GitHub credentials found for github.com"
        in notes
    )
    assert "GitHub collection made 0 requests" in notes
    assert "1 unresolved association lookups" in notes
    assert api.requests == []
    assert "Change (Alice)" in section(out, "Unmatched Commits")


def test_previous_release_is_the_nearest_published_one_before_a_historical_head(
    repo, env, api
):
    base = repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    repo.commit("Patch", "patch.txt")
    repo.git("tag", "v1.0.1")
    historical = repo.commit("Minor", "minor.txt")
    repo.git("tag", "v1.1.0")
    repo.commit("Major", "major.txt")
    repo.git("tag", "v2.0.0")
    first = f"/repos/{SLUG}/releases?per_page=100"
    api.add(
        first,
        [release("v2.0.0"), release("v1.1.0"), release("v1.2.0-rc.1", prerelease=True)],
        headers=api.link(first + "&page=2"),
    )
    api.add(
        first + "&page=2",
        [release("v1.0.1"), release("v1.0.0"), release("draft", draft=True)],
    )
    api.add(f"/repos/{SLUG}/releases/tags/v1.0.1", view("v1.0.1"))
    out, _, _ = run(repo, env, "--base", base, "--head", historical)
    assert "**Tag:** v1.0.1\n**Title:** Title of v1.0.1" in out
    assert "my GitHub release `v1.1.0`" in out
    assert "v2.0.0" not in section(out, "Previous Release (style reference)")
    assert api.paths("/releases/tags/") == [f"/repos/{SLUG}/releases/tags/v1.0.1"]


def test_explicit_previous_release_and_failed_lookup(repo, env, api):
    repo.commit("Initial", "README.md")
    api.add(
        f"/repos/{SLUG}/releases/tags/style%2Fexample", view("style/example", body=None)
    )
    out, _, _ = run(repo, env, "--previous-release", "style/example")
    assert "**Tag:** style/example" in out
    assert "*No description provided.*" in out
    assert "explicitly selected with --previous-release: style/example" in out
    assert api.paths("/releases?") == []

    out, _, _ = run(repo, env, "--previous-release", "missing")
    assert "*Previous release `missing` unavailable: release lookup failed.*" in out
    assert (
        "WARNING: could not fetch previous release missing; style reference omitted."
        in out
    )


def test_output_order_is_independent_of_completion_order(repo, env, api):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    shas = [repo.commit(f"Change {i}", f"f{i}.txt") for i in range(45)]
    api.associations = {sha: [pr(100 - i // 5)] for i, sha in enumerate(shas)}
    outputs = []
    for slow in (shas[0], shas[-1]):
        api.graphql_delay = {slow: 0.5}
        out, _, _ = run(repo, env, "--max-tokens", "0")
        outputs.append(out)
    assert outputs[0] == outputs[1]
    headings = re.findall(r"^### PR #(\d+)", outputs[0], flags=re.MULTILINE)
    assert headings == [str(100 - i) for i in range(9)]
    assert "45 commits; 9 merged PRs; 0 unmatched commits; 0 unresolved" in outputs[0]
    assert [method for method, _, _ in api.requests].count("POST") == 6
