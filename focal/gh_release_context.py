"""Collects merged pull requests and git commit histories between release tags."""

import re

from focal.utils import build_bot_exclusion_query, is_bot_author, run_gh_json, run_git


def get_release_context(
    tag_date: str,
    header_ref: str,
    tag_ref: str,
    head_ref: str = "HEAD",
    head_date: str | None = None,
) -> str:
    """Collects merged pull requests and git commit histories between release tags.

    Args:
        tag_date: The ISO date string of the base tag.
        header_ref: The header label for the base ref.
        tag_ref: The Git ref of the base tag.
        head_ref: The target head Git ref (defaults to "HEAD").
        head_date: Optional ISO date string for the head ref.

    Returns:
        Formatted markdown representation of PRs and commit history between tags.
    """
    # Filter out standard dependency bots at the query level to preserve token bandwidth
    search_query = f"is:pr is:merged base:main merged:>={tag_date} "
    if head_date:
        search_query += f"merged:<={head_date} "

    search_query += build_bot_exclusion_query()

    prs = run_gh_json(
        [
            "pr",
            "list",
            "--search",
            search_query,
            "--json",
            "number,title,body,author,labels,url",
            "--limit",
            "100",
        ]
    )

    parts = [f"# Release Context: {header_ref} to {head_ref}\n"]

    if not prs:
        parts.append("*No pull requests found matching the criteria.*")
    else:
        for pr in prs:
            number = pr.get("number", "Unknown")
            title = pr.get("title", "Untitled")
            author = pr.get("author", {}).get("login", "Unknown")
            url = pr.get("url", "")
            labels = [label.get("name") for label in pr.get("labels", [])]

            # Extract body and strip out hidden HTML comments
            raw_body = pr.get("body") or "*No description provided.*"
            clean_body = re.sub(r"<!--.*?-->", "", raw_body, flags=re.DOTALL).strip()

            if not clean_body:
                clean_body = "*No description provided.*"

            parts.append(f"## PR #{number}: {title}")
            parts.append(f"**Author:** @{author}")
            if labels:
                parts.append(f"**Labels:** {', '.join(labels)}")
            parts.append(f"**URL:** {url}")
            parts.append("\n**Intent / Description:**")
            parts.append(f"{clean_body}\n")
            parts.append("---\n")

    if tag_ref:
        code, log_out = run_git(
            [
                "log",
                f"{tag_ref}..{head_ref}",
                "--no-merges",
                "--format=* `%h` - %s (@%an)",
            ],
            check=False,
        )

        if code == 0 and log_out:
            commits = []
            for line in log_out.splitlines():
                line = line.strip()
                if line and not is_bot_author(line):
                    commits.append(line)

            if commits:
                parts.append("## Raw Commits\n")
                parts.extend(commits)
                parts.append("\n")

    return "\n".join(parts)
