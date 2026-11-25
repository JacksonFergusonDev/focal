"""`release-context` contracts for the local report, shared by the legacy dispatcher and
the Rust port.

GitHub is unreachable here. The legacy wrapper gets a `gh` stub that reports a refused
connection; the Rust binary gets a token and an API URL on a closed local port. Both
then build the report from local Git alone, so these tests pin release membership,
commit order, paths, pinned asset revisions, bot policy, focused diffs, project
background, and output bounds. Assertions check content and structure rather than
Git's byte-exact rendering (libgit2 pads diffstat columns differently).
"""

import os
import re
import socket
import subprocess
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ
EPOCH = datetime(2001, 1, 1, tzinfo=timezone.utc)


def closed_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


@pytest.fixture
def env(tmp_path):
    home = tmp_path / "home"
    home.mkdir()
    stub = tmp_path / "stub"
    stub.mkdir()
    gh = stub / "gh"
    gh.write_text(
        "#!/bin/sh\necho 'error connecting to api.github.com: connection refused' >&2\nexit 1\n"
    )
    gh.chmod(0o755)
    path = (
        str(stub)
        if os.environ.get("FOCAL_TEST_EMPTY_PATH")
        else f"{stub}:{os.environ['PATH']}"
    )
    return {
        "PATH": path,
        "HOME": str(home),
        "GIT_CONFIG_NOSYSTEM": "1",
        "LC_ALL": "C",
        "NO_COLOR": "1",
        "GH_TOKEN": "test-token",
        "FOCAL_GITHUB_API_URL": f"http://127.0.0.1:{closed_port()}",
    }


class Repo:
    """A Git repository whose commits get distinct, fixed dates."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.tick = 0
        root.mkdir(parents=True, exist_ok=True)
        self.git("init", "-q", "-b", "main")
        self.git("remote", "add", "origin", "https://github.com/octo/widget.git")

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


def run(cwd, env, *args):
    result = subprocess.run(
        [CLI, "release-context", *args],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        check=False,
    )
    return result.returncode, result.stdout.decode(), result.stderr.decode()


def report(cwd, env, *args):
    code, out, err = run(cwd, env, *args)
    assert code == 0, err
    assert out.startswith("<context>\n# Release Notes Request\n")
    assert out.endswith("\n</context>\n")
    return out


def section(out, heading):
    """The body of `## heading`, up to the next second-level heading."""
    return out.split(f"\n## {heading}")[1].split("\n## ")[0]


def balanced(text):
    """Whether every fenced block is closed, honoring longer nested fences."""
    open_fence = None
    for line in text.splitlines():
        match = re.match(r"(`{3,}|~{3,})(.*)$", line)
        if not match:
            continue
        marker, rest = match.groups()
        if open_fence is None:
            open_fence = marker
        elif (
            marker[0] == open_fence[0]
            and len(marker) >= len(open_fence)
            and not rest.strip()
        ):
            open_fence = None
    return open_fence is None


@pytest.fixture
def tagged(repo):
    """v1.0.0, v1.1.0, and v1.1.1 on main, plus a prerelease, a non-release tag, and a
    commit after the last release."""
    shas = {"v1.0.0": repo.commit("Initial", "README.md")}
    repo.git("tag", "v1.0.0")
    repo.commit("Feature", "feature.txt")
    shas["v1.1.0"] = repo.commit("Minor release", "minor.txt")
    repo.git("tag", "v1.1.0")
    shas["v1.1.1"] = repo.commit("Patch release", "patch.txt")
    repo.git("tag", "v1.1.1")
    repo.commit("Prerelease", "rc.txt")
    repo.git("tag", "v2.0.0-rc.1")
    repo.git("tag", "not-a-release")
    shas["later"] = repo.commit("Later work", "later.txt")
    return shas


@pytest.mark.parametrize(
    ("args", "label"),
    [
        ((), "v1.1.1"),
        (("patch",), "v1.1.1"),
        (("--patch",), "v1.1.1"),
        (("minor",), "v1.1.0"),
        (("--minor",), "v1.1.0"),
        (("-m",), "v1.1.0"),
        (("--since", "minor"), "v1.1.0"),
        (("--level", "major"), "v1.0.0"),
        (("major",), "v1.0.0"),
        (("--major",), "v1.0.0"),
        # The last level wins, whatever its form.
        (("major", "--patch"), "v1.1.1"),
        (("--minor", "major"), "v1.0.0"),
        (("--level", "major", "-m"), "v1.1.0"),
    ],
)
def test_level_forms_select_reachable_stable_tags(repo, tagged, env, args, label):
    out = report(repo.root, env, *args)
    assert f"# Release Context: {label} to HEAD\n" in out
    assert f"**Base:** {label} (`{tagged[label]}`, exclusive)" in out


def test_historical_release_from_a_newer_checkout(repo, tagged, env):
    out = report(repo.root, env, "--version", "v1.1.1")
    assert "# Release Context: v1.1.0 to v1.1.1\n" in out
    assert f"**Head:** `{tagged['v1.1.1']}` (inclusive)" in out
    assert "my GitHub release `v1.1.1`" in out
    assert "**Commits in range:** 1\n" in out
    assert "Patch release" in out
    assert "Later work" not in out
    assert "later.txt" not in out
    assert "rc.txt" not in out

    # An explicit head overrides the version tag.
    out = report(repo.root, env, "--version", "v1.1.1", "--head", "HEAD")
    assert "# Release Context: v1.1.1 to HEAD\n" in out
    assert "Later work" in out
    assert "Patch release" not in section(out, "Unmatched Commits")

    # Explicit historical endpoints, with the version inferred from the head tag.
    out = report(repo.root, env, "--base", "v1.0.0", "--head", "v1.1.0")
    assert "# Release Context: v1.0.0 to v1.1.0\n" in out
    assert "my GitHub release `v1.1.0`" in out
    assert "**Commits in range:** 2\n" in out
    assert "Patch release" not in out


def test_upcoming_version_drafts_from_head_and_untagged_heads_get_a_placeholder(
    repo, tagged, env
):
    out = report(repo.root, env, "--version", "v1.2.0")
    assert "# Release Context: v1.1.1 to HEAD\n" in out
    assert f"**Head:** `{tagged['later']}`" in out
    assert "my GitHub release `v1.2.0`" in out
    out = report(repo.root, env)
    assert "my GitHub release `vX.Y.Z`" in out
    assert "do not invent a version" in out


def test_root_release_includes_root_commits_and_pins_assets(repo, env):
    root = repo.commit("Root", "docs/demo space#1.png")
    repo.commit("Later", "later.gif")
    out = report(repo.root, env, "--head", root)
    assert f"# Release Context: Repository beginning to {root}\n" in out
    assert (
        "**Base:** Repository beginning (all reachable history, including root commits)"
        in out
    )
    assert "**Commits in range:** 1\n" in out
    unmatched = section(out, "Unmatched Commits")
    assert f"* `{root[:12]}` — Root (Alice)" in unmatched
    assert "  * `docs/demo space#1.png`" in unmatched
    assets = section(out, "Repository Assets (pinned to release commit)")
    assert f"Tracked image/video candidates at `{root}`" in assets
    assert (
        f"`<ASSET_URL: docs/demo space#1.png at {root}>` (repository URL unavailable)"
        in assets
    )
    assert "later.gif" not in out
    assert "docs/demo space#1.png" in section(out, "Release Impact")


def test_merge_range_keeps_topological_order_and_first_parent_paths(repo, env):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    repo.commit("Direct one", "one.txt")
    repo.git("checkout", "-qb", "feature")
    repo.commit("Feature a", "fa.txt")
    repo.commit("Feature b", "fb.txt")
    repo.git("checkout", "-q", "main")
    repo.commit("Direct two", "two.txt")
    repo.git("merge", "-q", "--no-ff", "feature", "-m", "Merge feature")
    expected = repo.git(
        "log", "--reverse", "--topo-order", "--format=%H", "v1.0.0..HEAD"
    ).split()
    out = report(repo.root, env, "--max-tokens", "0")
    unmatched = section(out, "Unmatched Commits")
    listed = re.findall(r"^\* `([0-9a-f]{12})`", unmatched, flags=re.MULTILINE)
    assert listed == [sha[:12] for sha in expected]
    merge = unmatched.split("Merge feature (Alice)\n")[1]
    assert merge.splitlines()[:2] == ["  * `fa.txt`", "  * `fb.txt`"]
    assert "**Commits in range:** 5\n" in out


def test_divergent_base_reports_set_difference(repo, env):
    repo.commit("Initial", "README.md")
    repo.git("checkout", "-qb", "other")
    base = repo.commit("Other", "other.txt")
    repo.git("checkout", "-q", "main")
    repo.commit("Head", "head.txt")
    out = report(repo.root, env, "--base", base)
    assert "Base is not an ancestor of head: this is Git set difference" in out
    assert "Other" not in section(out, "Unmatched Commits")
    assert "**Commits in range:** 1\n" in out


def test_shallow_history_warning(repo, env):
    root = repo.commit("Initial", "README.md")
    (repo.root / ".git/shallow").write_text(root + "\n")
    out = report(repo.root, env)
    assert (
        "WARNING: shallow checkout; available Git history and tag selection may be incomplete"
        in out
    )


def test_offline_output_keeps_local_evidence(repo, tagged, env):
    out = report(repo.root, env)
    assert "*Previous release unavailable: GitHub repository lookup failed.*" in out
    notes = section(out, "Completeness Notes")
    assert (
        "WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment."
        in notes
    )
    assert "WARNING: previous release style reference could not be fetched." in notes
    assert (
        "GitHub connection or service failure; remaining GitHub enrichment skipped."
        in notes
    )
    assert "*No merged PR associations found.*" in out
    assert "Later work (Alice)" in section(out, "Unmatched Commits")
    assert "later.txt" in section(out, "Release Impact")
    assert "Collected 2 commits; 0 merged PRs; 2 unmatched commits;" in notes
    headings = re.findall(r"^#+ .*$", out, flags=re.MULTILINE)
    assert headings == [
        "# Release Notes Request",
        "# Release Context: v1.1.1 to HEAD",
        "## Previous Release (style reference)",
        "## Repository Assets (pinned to release commit)",
        "## PR Index (all associated PRs)",
        "## Pull Requests",
        "## Unmatched Commits",
        "## Release Impact",
        "## Completeness Notes",
    ]


@pytest.mark.xfail(LEGACY, reason="legacy counts skipped lookups as empty", strict=True)
def test_offline_lookups_count_as_unresolved(repo, tagged, env):
    out = report(repo.root, env)
    notes = section(out, "Completeness Notes")
    assert "2 unresolved association lookups" in notes
    incomplete = notes.split("WARNING: PR enrichment is incomplete for: ")[1].split(
        "\n"
    )[0]
    assert f"`{tagged['later'][:12]}`" in incomplete
    assert "0 failed, 2 skipped" in notes


@pytest.mark.parametrize("mode", ["summary", "include", "exclude"])
def test_bot_policy_for_unmatched_commits(repo, env, mode):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    repo.commit("Human change", "app.txt")
    bot = repo.commit(
        "Bump library", "deps.txt", author="dependabot[bot] <bot@example.com>"
    )
    out = report(repo.root, env, "--bots", mode)
    unmatched = section(out, "Unmatched Commits")
    assert (bot[:12] in unmatched) == (mode == "include")
    assert ("## Bot Updates (compact summary)" in out) == (mode == "summary")
    if mode == "summary":
        assert f"* `{bot[:12]}` — Bump library (dependabot[bot])" in section(
            out, "Bot Updates (compact summary)"
        )
    assert "deps.txt" in section(out, "Release Impact")
    expected = {
        "summary": "Bot policy: summary; 0 bot PRs and 1 commits summarized.",
        "include": "Bot policy: include; 0 bot PRs and 1 commits included in full.",
        "exclude": "Bot policy: exclude; 0 bot PRs and 1 commits omitted from detail (still included in release impact).",
    }
    assert expected[mode] in out


def test_focused_diff_uses_endpoints_and_repeatable_paths(repo, env):
    base = repo.commit("Initial", "README.md")
    repo.commit("Wanted", "wanted.txt")
    repo.commit("Nested", "dir/nested.txt")
    repo.commit("Other", "other.txt")
    historical = repo.git("rev-parse", "HEAD")
    repo.commit("Not shipped", "wanted.txt")
    out = report(
        repo.root,
        env,
        "--base",
        base,
        "--head",
        historical,
        "--diff",
        "--path",
        "wanted.txt",
        "--path",
        "dir/",
    )
    focused = section(out, "Focused Diff")
    assert "Paths: wanted.txt, dir/" in focused
    assert "+Wanted" in focused
    assert "+Nested" in focused
    assert "other.txt" not in focused
    assert "Not shipped" not in out
    assert "other.txt" in section(out, "Release Impact")
    assert balanced(out)

    # Pathspecs resolve from the invocation directory, as with git diff.
    out = report(
        repo.root / "dir", env, "--base", base, "--diff", "--path", "nested.txt"
    )
    focused = section(out, "Focused Diff")
    assert "+Nested" in focused
    assert "wanted.txt" not in focused

    out = report(repo.root, env, "--base", base, "--diff", "--path", "missing.txt")
    assert "No matching changes." in section(out, "Focused Diff")
    out = report(repo.root, env, "--base", historical, "--head", historical)
    assert "No net file changes." in section(out, "Release Impact")
    assert "**Commits in range:** 0\n" in out


def test_project_context_appends_the_current_checkout(repo, tagged, env):
    (repo.root / "README.md").write_text("".join(f"line {i}\n" for i in range(1, 601)))
    (repo.root / "Cargo.toml").write_text('[package]\nname = "x"\n')
    (repo.root / "untracked.txt").write_text("new\n")
    out = report(repo.root, env, "--project-context", "--max-tokens", "0")
    background = out.split(
        "\n# Project Background (current checkout, not the release snapshot)\n\n"
    )[1]
    assert background.startswith(
        "Tree is limited to six levels; core files are limited to their first 500 lines.\n\n# Repository Tree\n```text\n"
    )
    assert "## Completeness Notes" in out.split("# Project Background")[0]
    headings = re.findall(r"^# .*$", background, flags=re.MULTILINE)
    assert headings == [
        "# Repository Tree",
        "# Git Info",
        "# README.md",
        "# Cargo.toml",
    ]
    assert "?? untracked.txt" in background
    assert "line 500\n```" in background
    assert "line 501" not in background
    assert '```toml\n[package]\nname = "x"\n```' in background
    assert balanced(out)


def test_project_context_shares_the_output_budget(repo, tagged, env):
    (repo.root / "README.md").write_text("x" * 200 + "\n" + "word " * 20000 + "\n")
    out = report(repo.root, env, "--project-context", "--max-tokens", "1000")
    body = out.removeprefix("<context>\n").removesuffix("\n</context>\n")
    assert len(body) <= 4000
    assert "## Output Limit" in body
    assert "output exceeded the approximate 1,000-token budget" in body
    assert balanced(body)
    assert "## Completeness Notes" in body


def test_output_budget_keeps_impact_and_counts(repo, env):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    for index in range(12):
        repo.commit(
            f"Change number {index} " + "detail " * 60, f"src/file{index:02}.txt"
        )
    out = report(repo.root, env, "--max-tokens", "2000", "--diff")
    body = out.removeprefix("<context>\n").removesuffix("\n</context>\n")
    assert len(body) <= 8000
    assert "## Output Limit" in body
    assert re.search(r"\* [\d,]+ characters of release detail omitted", body)
    # The aggregate impact and final counts move after the omission notice.
    tail = body.split("## Output Limit")[1]
    assert "## Release Impact" in tail
    assert "src/file11.txt" in tail
    assert "Collected 12 commits; 0 merged PRs; 12 unmatched commits" in tail
    assert balanced(body)

    full = report(repo.root, env, "--max-tokens", "0", "--diff")
    assert "## Output Limit" not in full
    assert "## PR Index" not in full
    assert full.count("Change number") == 24
    assert "+Change number 11" in section(full, "Focused Diff")


def test_compact_detail_marks_omitted_paths(repo, env):
    repo.commit("Initial", "README.md")
    repo.git("tag", "v1.0.0")
    for index in range(7):
        (repo.root / f"many{index}.txt").write_text("x\n")
    repo.commit("Wide change", "wide.txt")
    out = report(repo.root, env)
    assert "  * 3 additional changed paths omitted from detail." in section(
        out, "Unmatched Commits"
    )
    full = report(repo.root, env, "--max-tokens", "0")
    assert "additional changed paths omitted" not in full
    assert "  * `many6.txt`" in full


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (("bogus",), "unknown argument: bogus"),
        (("--level", "invalid"), "invalid release level 'invalid'"),
        (("--bots", "invalid"), "Invalid value for '--bots'"),
        (("--max-tokens", "500"), "--max-tokens must be 0 or at least 1000"),
        (("--path", "app.txt"), "--path requires --diff"),
    ],
)
def test_argument_errors(repo, env, args, message):
    repo.commit("Initial")
    code, out, err = run(repo.root, env, *args)
    assert code == 1
    assert out == ""
    assert message in err


def test_help_lists_options(repo, env):
    code, out, _ = run(repo.root, env, "--help")
    assert code == 0
    for option in [
        "--minor",
        "-m",
        "--major",
        "--patch",
        "--base",
        "--head",
        "--version",
        "--previous-release",
        "--bots",
        "--request-timeout",
        "--github-timeout",
        "--max-tokens",
        "--diff",
        "--path",
        "--project-context",
    ]:
        assert option in out


@pytest.mark.xfail(
    LEGACY, reason="clap reports usage errors with exit status 2", strict=True
)
@pytest.mark.parametrize(
    ("args", "message"),
    [
        (("--unknown-flag",), "unexpected argument '--unknown-flag'"),
        (("--base",), "a value is required for '--base <REF>'"),
    ],
)
def test_usage_errors_come_from_clap(repo, env, args, message):
    repo.commit("Initial")
    code, _, err = run(repo.root, env, *args)
    assert code == 2
    assert message in err


@pytest.mark.xfail(LEGACY, reason="legacy echoes the failing git command", strict=True)
def test_unknown_revision(repo, env):
    repo.commit("Initial")
    code, out, err = run(repo.root, env, "--head", "does-not-exist")
    assert code == 1
    assert out == ""
    assert "unknown revision or not a commit: 'does-not-exist'" in err
