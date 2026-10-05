"""Build release evidence from Git membership and commit-associated GitHub PRs."""

import re
from collections import defaultdict
from typing import Any

from focal.errors import die, print_info
from focal.release_github import ReleaseGitHub
from focal.release_notes import (
    previous_release_context,
    release_asset_context,
    release_notes_prompt,
)
from focal.release_output import limit_release_output
from focal.release_prs import collect_pr_associations
from focal.utils import is_bot_author, run_git

SEMVER_TAG = re.compile(r"^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


def _git(args: list[str]) -> str:
    """Run a required Git operation."""
    return run_git(args)[1]


def resolve_release_range(
    base_ref: str | None, head_ref: str, level: str
) -> tuple[str | None, str, str]:
    """Resolve immutable endpoints, selecting a reachable stable SemVer tag."""
    head = _git(["rev-parse", "--verify", "--end-of-options", f"{head_ref}^{{commit}}"])
    if base_ref:
        return (
            _git(
                ["rev-parse", "--verify", "--end-of-options", f"{base_ref}^{{commit}}"]
            ),
            head,
            base_ref,
        )

    candidates = []
    for tag in _git(["tag", "--merged", head]).splitlines():
        match = SEMVER_TAG.fullmatch(tag)
        if not match:
            continue
        _, minor, patch = map(int, match.groups())
        if (level == "minor" and patch != 0) or (
            level == "major" and (minor != 0 or patch != 0)
        ):
            continue
        # A tagged target describes that release, so compare with its predecessor.
        if _git(["rev-parse", f"{tag}^{{commit}}"]) != head:
            candidates.append(tag)
    if not candidates:
        return None, head, "Repository beginning"
    args = ["describe", "--tags", "--abbrev=0"]
    for tag in candidates:
        args.extend(["--match", tag])
    tag = _git([*args, head])
    return _git(["rev-parse", f"{tag}^{{commit}}"]), head, tag


def _range_commit_paths(revision: str) -> dict[str, list[str]]:
    """Collect paths for every commit with one Git invocation, using first-parent merge diffs."""
    output = _git(
        [
            "log",
            "--format=%x1e%H",
            "--name-only",
            "--diff-merges=first-parent",
            revision,
        ]
    )
    paths = {}
    for record in output.split("\x1e"):
        record = record.strip()
        if not record:
            continue
        sha, _, files = record.partition("\n")
        paths[sha] = [path for path in files.splitlines() if path]
    return paths


def get_release_context(
    base_ref: str | None = None,
    head_ref: str | None = None,
    level: str = "patch",
    bots: str = "summary",
    include_diff: bool = False,
    paths: tuple[str, ...] = (),
    version: str | None = None,
    previous_release: str | None = None,
    request_timeout: float = 10,
    github_timeout: float = 60,
    max_tokens: int = 24000,
) -> str:
    """Collect an exact Git range, associated PR intent, and release impact.

    Git determines membership; GitHub supplies optional PR metadata. Unknown or
    failed associations remain visible as unmatched commits and completeness notes.
    """
    if max_tokens != 0 and max_tokens < 1000:
        die("--max-tokens must be 0 (unlimited) or at least 1000")
    if level not in {"patch", "minor", "major"}:
        die(f"invalid release level: {level}")
    if bots not in {"summary", "include", "exclude"}:
        die(f"invalid bot mode: {bots}")
    explicit_head = head_ref is not None
    head_ref = head_ref or "HEAD"
    range_head_ref = head_ref
    if version and not explicit_head:
        version_tag = f"refs/tags/{version}"
        code, _ = run_git(
            ["rev-parse", "--verify", "--quiet", f"{version_tag}^{{commit}}"],
            check=False,
        )
        if code == 0:
            head_ref = version
            range_head_ref = version_tag
            print_info(f"Using existing version tag {version} as the release target")
        else:
            print_info(
                f"Version tag {version} is not available locally; drafting from HEAD"
            )
    base, head, label = resolve_release_range(base_ref, range_head_ref, level)
    if not version:
        tags = _git(["tag", "--points-at", head]).splitlines()
        version = (
            head_ref
            if head_ref in tags
            else next((tag for tag in tags if SEMVER_TAG.fullmatch(tag)), "vX.Y.Z")
        )
    revision = f"{base}..{head}" if base else head
    log = _git(
        ["log", "--reverse", "--topo-order", "--format=%H%x09%an%x09%s", revision]
    )
    commits = []
    for line in log.splitlines():
        sha, author, subject = line.split("\t", 2)
        commits.append({"sha": sha, "author": author, "subject": subject})

    parts = [
        release_notes_prompt(version, head),
        f"# Release Context: {label} to {head_ref}",
        f"**Base:** {label}"
        + (
            f" (`{base}`, exclusive)"
            if base
            else " (all reachable history, including root commits)"
        ),
        f"**Head:** `{head}` (inclusive)",
        f"**Commits in range:** {len(commits)}",
    ]
    notes = [
        "Membership is determined by Git ancestry. PR associations are enrichment; PR descriptions may have changed since the release.",
        "Only merged PRs returned for in-range commits are included; no default-branch or merge-date filter is applied.",
    ]
    if base:
        code, _ = run_git(["merge-base", "--is-ancestor", base, head], check=False)
        if code == 1:
            notes.append(
                "Base is not an ancestor of head: this is Git set difference, not a linear release interval."
            )
        elif code != 0:
            die("could not verify release ancestry")
    if _git(["rev-parse", "--is-shallow-repository"]) == "true":
        notes.append(
            "WARNING: shallow checkout; available Git history and tag selection may be incomplete. Fetch full history and tags."
        )

    print_info(
        f"Collecting release context for {len(commits)} commits ({label} to {head_ref})"
    )
    github = ReleaseGitHub(request_timeout, github_timeout)
    repo_info = github.request(["repo", "view", "--json", "nameWithOwner,url"])
    repo = repo_info.get("nameWithOwner") if isinstance(repo_info, dict) else None
    if not repo:
        notes.append(
            "WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment."
        )
    reference, reference_notes = previous_release_context(
        repo, head, version, previous_release, github=github
    )
    notes.extend(reference_notes)
    parts.extend(["\n## Previous Release (style reference)\n", reference])
    repo_url = repo_info.get("url") if isinstance(repo_info, dict) else None
    parts.extend(
        [
            "\n## Repository Assets (pinned to release commit)\n",
            release_asset_context(head, repo_url),
        ]
    )
    groups: dict[int, dict[str, Any]] = {}
    grouped: dict[int, list[dict[str, str]]] = defaultdict(list)
    unmatched = []
    failures = []
    multiple = 0
    associations = (
        collect_pr_associations(repo, [c["sha"] for c in commits], github)
        if repo
        else {}
    )
    for commit in commits:
        prs = associations.get(commit["sha"]) if repo else []
        if prs is None:
            failures.append(commit["sha"])
            prs = []
        merged = sorted(
            (pr for pr in prs if pr.get("merged_at")), key=lambda pr: pr["number"]
        )
        if not merged:
            unmatched.append(commit)
            continue
        # Prefer the actual merge/squash commit match, then a stable PR number.
        matching = [pr for pr in merged if pr.get("merge_commit_sha") == commit["sha"]]
        pr = (matching or merged)[0]
        if len(merged) > 1:
            multiple += 1
        if len(merged) > 1 and multiple <= 20:
            notes.append(
                f"Commit `{commit['sha'][:12]}` has multiple merged PR associations: "
                + ", ".join(f"#{item['number']}" for item in merged)
                + f"; grouped under #{pr['number']}."
            )
        number = pr["number"]
        groups[number] = pr
        grouped[number].append(commit)

    print_info("Formatting local commit paths and release impact")
    commit_paths = _range_commit_paths(revision)
    bot_groups = 0
    bot_commits = 0
    bot_rows = []
    if max_tokens:
        parts.append("\n## PR Index (all associated PRs)\n")
        parts.extend(
            f"* [#{number}: {pr.get('title', 'Untitled')}]({pr.get('html_url', '')}) — {len(grouped[number])} in-range commits"
            for number, pr in groups.items()
            if bots != "exclude"
            or not (
                is_bot_author((pr.get("user") or {}).get("login", ""))
                or (pr.get("user") or {}).get("type") == "Bot"
            )
        )
        notes.append(
            "Bounded output includes a PR index subject to the overall size limit; detailed descriptions, path lists, and commit lists may be abbreviated. Use --max-tokens 0 for full detail."
        )
    parts.append("\n## Pull Requests\n")
    for number, pr in groups.items():
        author = (pr.get("user") or {}).get("login", "Unknown")
        entries = grouped[number]
        bot = is_bot_author(author) or (pr.get("user") or {}).get("type") == "Bot"
        changed = sorted(
            {path for c in entries for path in commit_paths.get(c["sha"], [])}
        )
        if bot:
            bot_groups += 1
            bot_commits += len(entries)
            bot_rows.append(
                f"* PR #{number}: {pr.get('title', 'Untitled')} — {pr.get('html_url', '')}; {len(entries)} commits, {len(changed)} paths"
            )
            if bots != "include":
                continue
        body = re.sub(r"<!--.*?-->", "", pr.get("body") or "", flags=re.DOTALL).strip()
        if max_tokens and len(body) > 800:
            original_length = len(body)
            excerpt = body[:800]
            boundary = excerpt.rfind("\n")
            excerpt = (
                excerpt[:boundary] if boundary > 400 else excerpt.rsplit(" ", 1)[0]
            )
            body = (
                excerpt
                + f"\n\n*[Description excerpt; {original_length:,} characters in full description.]*"
            )
        parts.extend(
            [
                f"### PR #{number}: {pr.get('title', 'Untitled')}",
                f"**Author:** @{author}",
                f"**URL:** {pr.get('html_url', '')}",
            ]
        )
        labels = [item["name"] for item in pr.get("labels", [])]
        if labels:
            parts.append(f"**Labels:** {', '.join(labels)}")
        parts.extend(
            [
                "\n**Intent / Description:**",
                body or "*No description provided.*",
                "\n**In-range commits:**",
            ]
        )
        parts.extend(
            f"* `{c['sha'][:12]}` — {c['subject']} ({c['author']})"
            for c in (entries[:5] if max_tokens else entries)
        )
        if max_tokens and len(entries) > 5:
            parts.append(
                f"* {len(entries) - 5} additional in-range commits omitted from detail."
            )
        parts.append("\n**Changed paths (in-range commits):**")
        parts.extend(f"* `{path}`" for path in (changed[:5] if max_tokens else changed))
        if max_tokens and len(changed) > 5:
            parts.append(
                f"* {len(changed) - 5} additional changed paths omitted from detail."
            )
    if not groups:
        parts.append("*No merged PR associations found.*")
    elif len(groups) == bot_groups and bots != "include":
        parts.append("*All associated PRs are covered by the bot policy below.*")

    parts.append("\n## Unmatched Commits\n")
    visible_unmatched = 0
    for commit in unmatched:
        if is_bot_author(commit["author"]):
            bot_commits += 1
            bot_rows.append(
                f"* `{commit['sha'][:12]}` — {commit['subject']} ({commit['author']})"
            )
            if bots != "include":
                continue
        visible_unmatched += 1
        parts.append(
            f"* `{commit['sha'][:12]}` — {commit['subject']} ({commit['author']})"
        )
        changed = commit_paths.get(commit["sha"], [])
        parts.extend(
            f"  * `{path}`" for path in (changed[:5] if max_tokens else changed)
        )
        if max_tokens and len(changed) > 5:
            parts.append(
                f"  * {len(changed) - 5} additional changed paths omitted from detail."
            )
    if not visible_unmatched:
        parts.append("*None under the selected bot policy.*")
    if bots == "summary" and bot_rows:
        parts.extend(["\n## Bot Updates (compact summary)\n", *bot_rows])

    # An empty-tree base includes every root commit rather than excluding one.
    diff_base = base or _git(["hash-object", "-t", "tree", "/dev/null"])
    parts.extend(
        [
            "\n## Release Impact\n",
            "Net changes between the resolved endpoints (including bot updates):",
            "```text",
            _git(["diff", "--no-ext-diff", "--stat", diff_base, head])
            or "No net file changes.",
            "```",
        ]
    )
    if include_diff:
        parts.extend(
            [
                "\n## Focused Diff\n",
                "Paths: " + (", ".join(paths) if paths else "all paths"),
                "```diff",
                _git(
                    [
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        diff_base,
                        head,
                        "--",
                        *paths,
                    ]
                )
                or "No matching changes.",
                "```",
            ]
        )
    notes.extend(
        [
            f"Collected {len(commits)} commits; {len(groups)} merged PRs; {len(unmatched)} unmatched commits; {len(failures)} unresolved association lookups; {multiple} ambiguous associations.",
            f"Bot policy: {bots}; {bot_groups} bot PRs and {bot_commits} commits "
            + (
                "included in full."
                if bots == "include"
                else "summarized."
                if bots == "summary"
                else "omitted from detail (still included in release impact)."
            ),
            "PR association pages are fetched until exhausted; no PR count ceiling. Output size limits are reported explicitly when detail is omitted.",
            "Unmatched commits may be direct pushes, cherry-picks, or commits without a GitHub association; no PR attribution is inferred from commit subjects.",
            "Tag selection uses locally available, reachable stable vMAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH tags; fetch tags before collecting a release.",
        ]
    )
    if multiple > 20:
        notes.append(
            f"Ambiguity details limited to 20 commits; {multiple - 20} additional ambiguous commits are counted above."
        )
    if failures:
        notes.append(
            "WARNING: PR enrichment is incomplete for: "
            + ", ".join(f"`{sha[:12]}`" for sha in failures[:20])
            + (f" (and {len(failures) - 20} more)" if len(failures) > 20 else "")
        )
    notes.extend(github.notes)
    notes.append(
        f"GitHub collection made {github.requests} requests; per-request timeout {request_timeout:g}s, total GitHub budget {github_timeout:g}s. Unknown associations include failed or skipped lookups."
    )
    parts.extend(["\n## Completeness Notes\n", *(f"* {note}" for note in notes)])
    return limit_release_output("\n".join(parts) + "\n", max_tokens)
