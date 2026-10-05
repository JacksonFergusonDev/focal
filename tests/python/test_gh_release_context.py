import subprocess
from contextlib import contextmanager
from pathlib import Path
from typing import Any
from unittest.mock import patch

import pytest

from focal.gh_release_context import (
    get_release_context,
    resolve_release_range,
)
from focal.release_github import ReleaseGitHub
from focal.release_notes import previous_release_context, release_asset_context
from focal.release_prs import associated_prs as _associated_prs


@pytest.fixture
def repo(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)

    def git(*args):
        return subprocess.check_output(["git", *args], text=True).strip()

    git("init", "-b", "main")
    git("config", "user.name", "Alice")
    git("config", "user.email", "alice@example.com")

    def commit(subject, filename="app.txt", author=None):
        (tmp_path / filename).parent.mkdir(parents=True, exist_ok=True)
        with (tmp_path / filename).open("a") as file:
            file.write(subject + "\n")
        git("add", "--", filename)
        args = ["commit", "-m", subject]
        if author:
            args.extend(["--author", f"{author} <bot@example.com>"])
        git(*args)
        return git("rev-parse", "HEAD")

    return git, commit


def pr(number=1, author="alice", **kwargs):
    return {
        "number": number,
        "title": "Fix bug",
        "user": {"login": author},
        "html_url": f"https://github.com/org/repo/pull/{number}",
        "labels": [{"name": "bug"}],
        "body": "Fixes #1\n<!-- hidden info -->",
        "merged_at": "2026-01-01T00:00:00Z",
        "base": {"ref": "release/1.0"},
        **kwargs,
    }


@contextmanager
def github(associations, releases=(), release_views=None):
    def response(self, args, **kwargs):
        if args[0] == "repo":
            return {"nameWithOwner": "org/repo", "url": "https://github.com/org/repo"}
        if args[0] == "release":
            return (release_views or {}).get(args[2])
        if "/releases?" in args[1]:
            return list(releases)
        if args[1] == "graphql":
            import re

            shas = re.findall(r'object\(oid: "([^"]+)"\)', args[-1])
            data: dict[str, Any] = {}
            for i, sha in enumerate(shas):
                prs = associations.get(sha, [])
                if prs is None:
                    data[f"c{i}"] = None
                    continue
                nodes = [
                    {
                        "number": p["number"],
                        "title": p["title"],
                        "body": p["body"],
                        "url": p["html_url"],
                        "mergedAt": p["merged_at"],
                        "mergeCommit": {"oid": p.get("merge_commit_sha")},
                        "author": {
                            "login": p["user"]["login"],
                            "__typename": p["user"].get("type", "User"),
                        },
                        "labels": {
                            "nodes": p["labels"],
                            "pageInfo": {"hasNextPage": False},
                        },
                    }
                    for p in prs
                ]
                data[f"c{i}"] = {
                    "associatedPullRequests": {
                        "nodes": nodes,
                        "pageInfo": {"hasNextPage": False},
                    }
                }
            return {"data": {"repository": data}}
        sha = args[1].split("/commits/")[1].split("/")[0]
        return associations.get(sha, [])

    with patch(
        "focal.release_github.ReleaseGitHub.request",
        autospec=True,
        side_effect=response,
    ) as api:
        yield api


def test_exact_membership_groups_commits_without_default_branch_filter(repo):
    git, commit = repo
    base = commit("Initial")
    git("tag", "v1.0.0")
    first = commit("Fix one", "one.txt")
    second = commit("Fix two", "two.txt")
    direct = commit("Direct fix", "direct.txt")
    with github({first: [pr()], second: [pr()]}):
        output = get_release_context()
    assert f"`{base}`, exclusive" in output
    assert f"**Head:** `{direct}`" in output
    assert output.count("### PR #1:") == 1
    assert "hidden info" not in output
    assert "Fixes #1" in output
    assert "`one.txt`" in output
    assert "`two.txt`" in output
    assert output.count(f"`{first[:12]}`") == 1
    assert output.count(f"`{second[:12]}`") == 1
    unmatched = output.split("## Unmatched Commits")[1].split("## Release Impact")[0]
    assert "Direct fix" in unmatched
    assert "Fix one" not in unmatched
    assert "3 commits; 1 merged PRs; 1 unmatched commits" in output


def test_historical_head_and_cherry_pick(repo):
    git, commit = repo
    base = commit("Initial")
    git("checkout", "-b", "feature")
    original = commit("Backportable fix", "fix.txt")
    git("checkout", "main")
    commit("Release change")
    git("cherry-pick", original)
    historical = git("rev-parse", "HEAD")
    later = commit("Not shipped", "later.txt")
    with github({historical: [pr(2, merge_commit_sha=original)]}):
        output = get_release_context(base, historical)
    assert "Backportable fix" in output
    assert "Not shipped" not in output
    assert later[:12] not in output
    assert "### PR #2:" in output


def test_merge_commit_and_paths_are_included(repo):
    git, commit = repo
    base = commit("Initial")
    git("checkout", "-b", "feature")
    feature = commit("Feature", "feature.txt")
    git("checkout", "main")
    commit("Direct", "direct.txt")
    git("merge", "--no-ff", "feature", "-m", "Merge feature")
    merge = git("rev-parse", "HEAD")
    with github({feature: [pr()], merge: [pr(merge_commit_sha=merge)]}):
        output = get_release_context(base)
    assert "Merge feature" in output
    assert "`feature.txt`" in output
    assert "3 commits; 1 merged PRs" in output
    pr_section = output.split("### PR #1:")[1].split("## Unmatched Commits")[0]
    assert "`direct.txt`" not in pr_section


def test_no_tags_includes_root_and_uses_selected_head(repo):
    _, commit = repo
    root = commit("Root", "root.txt")
    later = commit("Later", "later.txt")
    with github({}):
        output = get_release_context(head_ref=root)
    assert "including root commits" in output
    assert "Root" in output
    assert "root.txt" in output
    assert later[:12] not in output
    assert "later.txt" not in output


def test_stable_semver_tag_selection_and_tagged_head(repo):
    git, commit = repo
    initial = commit("Major")
    git("tag", "v1.0.0")
    patch_commit = commit("Patch")
    git("tag", "v1.0.1")
    minor = commit("Minor")
    git("tag", "v1.1.0")
    commit("Prerelease")
    git("tag", "v2.0.0-rc.1")
    git("tag", "not-a-release")
    head = commit("Latest")
    assert resolve_release_range(None, head, "patch") == (minor, head, "v1.1.0")
    assert resolve_release_range(None, head, "minor")[0] == minor
    assert resolve_release_range(None, head, "major")[0] == initial
    assert resolve_release_range(None, minor, "patch")[0] == patch_commit
    assert resolve_release_range(None, minor, "minor")[0] == initial


@pytest.mark.parametrize("mode", ["summary", "include", "exclude"])
def test_bot_modes_preserve_net_impact(repo, mode):
    _, commit = repo
    base = commit("Initial")
    bot = commit("Update library", "deps.txt", author="dependabot[bot]")
    direct_bot = commit("Update workflow", "ci.txt", author="github-actions[bot]")
    with github({bot: [pr(author="dependabot[bot]", body="Verbose bot description")]}):
        output = get_release_context(base, bots=mode)
    assert "deps.txt" in output.split("## Release Impact")[1]
    assert "ci.txt" in output.split("## Release Impact")[1]
    assert "1 bot PRs and 2 commits" in output
    assert ("Verbose bot description" in output) == (mode == "include")
    assert ("Bot Updates (compact summary)" in output) == (mode == "summary")
    detail = output.split("## Release Impact")[0].split("## Unmatched Commits")[1]
    assert (direct_bot[:12] in detail) == (mode != "exclude")


def test_focused_diff_uses_release_endpoints(repo):
    _, commit = repo
    base = commit("Initial")
    commit("Wanted", "wanted.txt")
    commit("Other", "other.txt")
    with github({}):
        output = get_release_context(base, include_diff=True, paths=("wanted.txt",))
    diff = output.split("## Focused Diff")[1].split("## Completeness Notes")[0]
    assert "+Wanted" in diff
    assert "other.txt" not in diff
    assert (
        "other.txt" in output.split("## Release Impact")[1].split("## Focused Diff")[0]
    )


def test_failed_enrichment_is_visible_and_keeps_commits(repo):
    _, commit = repo
    base = commit("Initial")
    sha = commit("Fix")
    with github({sha: None}):
        output = get_release_context(base)
    assert "Fix" in output
    assert "1 unresolved association lookups" in output
    assert "WARNING: PR enrichment is incomplete" in output
    with patch("focal.release_github.ReleaseGitHub.request", return_value=None):
        output = get_release_context(base)
    assert "repository lookup failed" in output
    assert "Fix" in output


def test_pagination_has_no_100_pr_ceiling():
    page_one = [pr(number=i) for i in range(1, 101)]
    with patch(
        "focal.release_github.ReleaseGitHub.request", side_effect=[page_one, [pr(101)]]
    ) as api:
        result = _associated_prs("org/repo", "abc", ReleaseGitHub())
    assert result is not None
    assert len(result) == 101
    assert "page=2" in api.call_args_list[1].args[0][1]
    with patch(
        "focal.release_github.ReleaseGitHub.request", side_effect=[page_one, None]
    ):
        assert _associated_prs("org/repo", "abc", ReleaseGitHub()) is None


def test_ambiguous_associations_prefer_merge_match_and_ignore_open_prs(repo):
    _, commit = repo
    base = commit("Initial")
    sha = commit("Fix")
    with github({sha: [pr(1), pr(2, merge_commit_sha=sha), pr(3, merged_at=None)]}):
        output = get_release_context(base)
    assert "### PR #2:" in output
    assert "### PR #1:" not in output
    assert "#1, #2; grouped under #2" in output
    assert "1 ambiguous associations" in output


def test_empty_range_fetches_style_but_not_commit_associations(repo):
    _, commit = repo
    sha = commit("Initial")
    with github({}) as api:
        output = get_release_context(sha, sha)
    assert all("/commits/" not in str(call) for call in api.call_args_list)
    assert "0 commits" in output
    assert "No net file changes" in output


def test_shallow_history_warning(repo):
    git, commit = repo
    sha = commit("Initial")
    (Path(git("rev-parse", "--git-dir")) / "shallow").write_text(sha + "\n")
    with github({}):
        output = get_release_context()
    assert "WARNING: shallow checkout" in output


def test_divergent_base_reports_set_difference(repo):
    git, commit = repo
    commit("Initial")
    git("checkout", "-b", "other")
    base = commit("Other", "other.txt")
    git("checkout", "main")
    commit("Head", "head.txt")
    with github({}):
        output = get_release_context(base)
    assert "Base is not an ancestor" in output
    assert (
        "Other"
        not in output.split("## Unmatched Commits")[1].split("## Release Impact")[0]
    )


def test_invalid_ref_fails(repo):
    _, commit = repo
    commit("Initial")
    with pytest.raises(SystemExit):
        get_release_context("does-not-exist")


def test_more_than_100_distinct_prs_are_rendered():
    shas = [f"{i:040x}" for i in range(1, 102)]

    def git_output(args):
        if args[0] == "tag":
            return ""
        if args[0] == "log" and "--name-only" in args:
            return "".join(f"\x1e{sha}\napp.txt\n" for sha in shas)
        if args[0] == "log":
            return "\n".join(f"{sha}\tAlice\tFix {i}" for i, sha in enumerate(shas, 1))
        if args[0] == "rev-parse":
            return "false"
        if args[0] == "rev-list":
            return args[-1] + " parent"
        if args[0] == "diff-tree":
            return "app.txt"
        if args[0] == "diff":
            return "app.txt | 101 +"
        raise AssertionError(args)

    with (
        patch(
            "focal.gh_release_context.resolve_release_range",
            return_value=("base", "head", "v1.0.0"),
        ),
        patch("focal.gh_release_context._git", side_effect=git_output),
        patch("focal.gh_release_context.run_git", return_value=(0, "")),
        github({sha: [pr(i)] for i, sha in enumerate(shas, 1)}),
        patch(
            "focal.gh_release_context.release_asset_context", return_value="No assets"
        ),
        patch(
            "focal.gh_release_context.previous_release_context",
            return_value=("No previous release", []),
        ),
    ):
        output = get_release_context()
    assert output.count("### PR #") == 101
    assert "101 commits; 101 merged PRs" in output


def test_unreachable_tag_is_not_selected(repo):
    git, commit = repo
    initial = commit("Initial")
    git("tag", "v1.0.0")
    git("checkout", "-b", "other")
    commit("Other", "other.txt")
    git("tag", "v9.0.0")
    git("checkout", "main")
    head = commit("Head")
    assert resolve_release_range(None, head, "patch") == (initial, head, "v1.0.0")


def test_cherry_pick_without_association_stays_unmatched(repo):
    git, commit = repo
    base = commit("Initial")
    git("checkout", "-b", "feature")
    original = commit("Fix (#42)", "fix.txt")
    git("checkout", "main")
    commit("Release", "release.txt")
    git("cherry-pick", original)
    with github({original: [pr(42)]}):
        output = get_release_context(base)
    assert "Fix (#42)" in output
    assert "### PR #42:" not in output
    assert "no PR attribution is inferred" in output


def release_entry(tag, **kwargs):
    return {"tag_name": tag, "draft": False, "prerelease": False, **kwargs}


def release_view(tag, **kwargs):
    return {
        "tagName": tag,
        "name": "A polished release title",
        "body": "## Highlights\nPrevious features\n![Old demo](https://example.com/main/demo.svg)",
        "url": f"https://github.com/org/repo/releases/tag/{tag}",
        **kwargs,
    }


def test_embedded_prompt_version_and_previous_release_title_and_body(repo):
    git, commit = repo
    base = commit("Initial")
    git("tag", "v1.0.0")
    head = commit("New feature")
    with github({}, [release_entry("v1.0.0")], {"v1.0.0": release_view("v1.0.0")}):
        output = get_release_context(base, version="v1.1.0")
    assert output.startswith("# Release Notes Request")
    assert "my GitHub release `v1.1.0`" in output
    assert f"targeting commit `{head}`" in output
    assert "**Title:** A polished release title" in output
    assert "## Highlights\nPrevious features" in output
    assert "Use it for style only" in output
    assert "do not reuse unpinned links from the previous release" in output
    assert "separate checklist after the draft" in output


def test_version_inferred_from_target_tag_otherwise_placeholder(repo):
    git, commit = repo
    commit("Initial")
    git("tag", "v1.0.0")
    with github({}):
        tagged = get_release_context(head_ref="v1.0.0")
    assert "my GitHub release `v1.0.0`" in tagged
    commit("Unreleased")
    with github({}):
        untagged = get_release_context()
    assert "my GitHub release `vX.Y.Z`" in untagged
    assert "do not invent a version" in untagged


def test_previous_release_is_before_historical_head_and_independent_of_base(repo):
    git, commit = repo
    base = commit("Initial")
    git("tag", "v1.0.0")
    commit("Patch")
    git("tag", "v1.0.1")
    historical = commit("Minor")
    git("tag", "v1.1.0")
    commit("Future")
    git("tag", "v2.0.0")
    entries = [release_entry(tag) for tag in ("v2.0.0", "v1.1.0", "v1.0.1", "v1.0.0")]
    with github({}, entries, {"v1.0.1": release_view("v1.0.1")}):
        output = get_release_context(base, historical)
    assert "**Tag:** v1.0.1" in output
    assert "**Base:** " + base in output
    assert "**Tag:** v2.0.0" not in output
    assert "**Tag:** v1.1.0" not in output


def test_previous_release_skips_drafts_and_prereleases(repo):
    git, commit = repo
    commit("Stable")
    git("tag", "v1.0.0")
    commit("Prerelease")
    git("tag", "v1.1.0-rc.1")
    commit("Draft")
    git("tag", "v1.1.0")
    head = commit("Upcoming")
    entries = [
        release_entry("v1.1.0", draft=True),
        release_entry("v1.1.0-rc.1", prerelease=True),
        release_entry("v1.0.0"),
    ]
    with github({}, entries, {"v1.0.0": release_view("v1.0.0")}):
        reference, _ = previous_release_context("org/repo", head, "v1.2.0")
    assert "**Tag:** v1.0.0" in reference


def test_explicit_previous_release_does_not_need_local_tag(repo):
    _, commit = repo
    head = commit("Upcoming")
    with github({}, release_views={"style-example": release_view("style-example")}):
        reference, notes = previous_release_context(
            "org/repo", head, "v1.0.0", "style-example"
        )
    assert "**Tag:** style-example" in reference
    assert "explicitly selected" in " ".join(notes)


def test_previous_release_failures_and_missing_description(repo):
    _, commit = repo
    head = commit("Upcoming")
    with patch("focal.release_github.ReleaseGitHub.request", return_value=None):
        reference, notes = previous_release_context("org/repo", head, "v1.0.0")
        assert "listing failed" in reference
        assert "WARNING" in " ".join(notes)
        reference, notes = previous_release_context(
            "org/repo", head, "v1.0.0", "missing"
        )
        assert "lookup failed" in reference
        assert "WARNING" in " ".join(notes)
    with github({}, release_views={"example": release_view("example", body=None)}):
        reference, _ = previous_release_context("org/repo", head, "v1.0.0", "example")
    assert "No description provided" in reference


def test_previous_release_pagination_reaches_older_release(repo):
    git, commit = repo
    commit("Stable")
    git("tag", "v1.0.0")
    head = commit("Upcoming")
    pages = [
        [release_entry(f"unavailable-{i}") for i in range(100)],
        [release_entry("v1.0.0")],
        release_view("v1.0.0"),
    ]
    with patch("focal.release_github.ReleaseGitHub.request", side_effect=pages) as api:
        reference, notes = previous_release_context("org/repo", head, "v1.1.0")
    assert "**Tag:** v1.0.0" in reference
    assert "page=2" in api.call_args_list[1].args[0][1]
    assert "100 published release tags were not reachable" in " ".join(notes)


def test_asset_inventory_uses_head_tree_and_encoded_commit_pinned_urls(repo):
    git, commit = repo
    commit("Image", "docs/demo space#1.SVG")
    head = commit("Video", "demos/demo.mp4")
    commit("Future image", "later.png")
    Path("untracked.gif").write_text("Untracked")
    # Also ignore media-looking symlinks.
    Path("link.svg").symlink_to("later.png")
    git("add", "link.svg")
    git("commit", "-m", "Symlink")
    output = release_asset_context(head, "https://git.example.test/org/repo")
    assert (
        f"https://git.example.test/org/repo/blob/{head}/docs/demo%20space%231.SVG"
        in output
    )
    assert f"https://git.example.test/org/repo/raw/{head}/demos/demo.mp4" in output
    assert "later.png" not in output
    assert "untracked.gif" not in output
    assert "link.svg" not in release_asset_context(git("rev-parse", "HEAD"), None)
    assert "have not been verified" in output
    assert "/main/" not in output
    placeholder = release_asset_context(head, None)
    assert f"<ASSET_URL: demos/demo.mp4 at {head}>" in placeholder


def test_compact_descriptions_paths_and_commits_are_marked(repo):
    _, commit = repo
    base = commit("Initial")
    shas = [commit(f"Change {i}", f"file{i}.txt") for i in range(7)]
    large_pr = pr(body="Long description " * 200)
    with github({sha: [large_pr] for sha in shas}):
        compact = get_release_context(base)
    assert "Description excerpt" in compact
    assert "2 additional changed paths omitted" in compact
    assert "2 additional in-range commits omitted" in compact
    assert "## PR Index" in compact
    with github({sha: [large_pr] for sha in shas}):
        full = get_release_context(base, max_tokens=0)
    assert "Description excerpt" not in full
    assert ("Long description " * 200).rstrip() in full
    assert "additional changed paths omitted" not in full


def test_version_tag_selects_release_range_even_after_head_moves(repo):
    git, commit = repo
    base = commit("Previous release")
    git("tag", "v0.9.0")
    shipped = commit("Shipped feature", "shipped.txt")
    git("tag", "v0.10.0")
    later = commit("After release", "later.txt")
    with github({}, release_views={"v0.9.0": release_view("v0.9.0")}):
        output = get_release_context(version="v0.10.0", previous_release="v0.9.0")
    assert "# Release Context: v0.9.0 to v0.10.0" in output
    assert f"**Base:** v0.9.0 (`{base}`, exclusive)" in output
    assert f"**Head:** `{shipped}`" in output
    assert "Shipped feature" in output
    assert "After release" not in output
    assert "later.txt" not in output
    assert later[:12] not in output


def test_explicit_head_overrides_existing_version_tag(repo):
    git, commit = repo
    commit("Previous release")
    git("tag", "v0.9.0")
    commit("Shipped feature")
    git("tag", "v0.10.0")
    later = commit("After release")
    with github({}):
        output = get_release_context(head_ref="HEAD", version="v0.10.0")
    assert "# Release Context: v0.10.0 to HEAD" in output
    assert f"**Head:** `{later}`" in output
    assert "After release" in output
    assert "Shipped feature" not in output


def test_upcoming_version_without_local_tag_uses_head(repo):
    git, commit = repo
    commit("Previous release")
    git("tag", "v0.9.0")
    upcoming = commit("Upcoming feature")
    with github({}):
        output = get_release_context(version="v0.10.0")
    assert "# Release Context: v0.9.0 to HEAD" in output
    assert f"**Head:** `{upcoming}`" in output
    assert "Upcoming feature" in output


def test_version_uses_tag_when_branch_has_the_same_name(repo):
    git, commit = repo
    commit("Previous release")
    git("tag", "v0.9.0")
    shipped = commit("Shipped feature")
    git("tag", "v0.10.0")
    commit("Later")
    git("branch", "v0.10.0")
    with github({}):
        output = get_release_context(version="v0.10.0", base_ref="v0.9.0")
    assert f"**Head:** `{shipped}`" in output
    assert "Later" not in output
