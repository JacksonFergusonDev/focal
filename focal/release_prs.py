"""Batch commit-to-PR enrichment while retaining complete association pagination."""

import json
from typing import Any

from focal.errors import print_info
from focal.release_github import ReleaseGitHub

BATCH_SIZE = 20


def associated_prs(
    repo: str, sha: str, github: ReleaseGitHub
) -> list[dict[str, Any]] | None:
    """Fetch all REST pages for an association too large for the batch query."""
    prs: list[dict[str, Any]] = []
    page = 1
    while True:
        result = github.request(
            ["api", f"repos/{repo}/commits/{sha}/pulls?per_page=100&page={page}"]
        )
        if not isinstance(result, list):
            return None
        prs.extend(result)
        if len(result) < 100:
            return prs
        page += 1


def _normalize(pr: dict[str, Any]) -> dict[str, Any]:
    """Translate GraphQL metadata to the shared PR formatting contract."""
    author = pr.get("author") or {}
    return {
        "number": pr["number"],
        "title": pr["title"],
        "body": pr.get("body"),
        "html_url": pr["url"],
        "merged_at": pr.get("mergedAt"),
        "merge_commit_sha": (pr.get("mergeCommit") or {}).get("oid"),
        "user": {
            "login": author.get("login", "Unknown"),
            "type": author.get("__typename"),
        },
        "labels": (pr.get("labels") or {}).get("nodes", []),
    }


def collect_pr_associations(
    repo: str, shas: list[str], github: ReleaseGitHub
) -> dict[str, list[dict[str, Any]] | None]:
    """Collect twenty commits per request, preserving failures as unknown evidence."""
    associations: dict[str, list[dict[str, Any]] | None] = dict.fromkeys(shas)
    owner, name = repo.split("/", 1)
    for offset in range(0, len(shas), BATCH_SIZE):
        if github.stopped:
            break
        batch = shas[offset : offset + BATCH_SIZE]
        print_info(
            f"Resolving PRs for commits {offset + 1}-{offset + len(batch)} of {len(shas)}"
        )
        fields = []
        for index, sha in enumerate(batch):
            fields.append(f"""c{index}: object(oid: {json.dumps(sha)}) {{ ... on Commit {{
                associatedPullRequests(first: 100) {{
                    pageInfo {{ hasNextPage }}
                    nodes {{ number title body url mergedAt mergeCommit {{ oid }}
                        author {{ login __typename }}
                        labels(first: 100) {{ nodes {{ name }} pageInfo {{ hasNextPage }} }}
                    }}
                }}
            }} }}""")
        query = f"query {{ repository(owner: {json.dumps(owner)}, name: {json.dumps(name)}) {{ {' '.join(fields)} }} rateLimit {{ remaining resetAt }} }}"
        result = github.request(["api", "graphql", "-f", f"query={query}"])
        data = (
            (result.get("data") or {}).get("repository")
            if isinstance(result, dict)
            else None
        )
        if not isinstance(data, dict):
            # A query/schema failure gets one bounded REST fallback per commit.
            # Rate limits, hangs, and authentication failures open the circuit.
            for sha in batch:
                if github.stopped:
                    break
                associations[sha] = associated_prs(repo, sha, github)
            continue
        for index, sha in enumerate(batch):
            commit = data.get(f"c{index}")
            connection = (
                commit.get("associatedPullRequests")
                if isinstance(commit, dict)
                else None
            )
            if not isinstance(connection, dict) or not isinstance(
                connection.get("nodes"), list
            ):
                # A missing remote commit (e.g. unpushed work) remains unknown.
                continue
            nodes = connection["nodes"]
            if any(not isinstance(pr, dict) for pr in nodes):
                continue
            overflow = connection.get("pageInfo", {}).get("hasNextPage") or any(
                (pr.get("labels") or {}).get("pageInfo", {}).get("hasNextPage")
                for pr in nodes
            )
            associations[sha] = (
                associated_prs(repo, sha, github)
                if overflow
                else [_normalize(pr) for pr in nodes]
            )
    return associations
