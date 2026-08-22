"""Fetches and formats GitHub issues and discussion threads into markdown."""

from focal.utils import run_gh_json


def process_issue(issue_id: str) -> str:
    """Fetch and format a GitHub issue as markdown.

    Uses the `gh` CLI to retrieve the issue title, body, comments, and URL,
    and returns a markdown-formatted string containing the issue details
    and its discussion thread.

    Args:
        issue_id: The issue number or identifier to fetch (as accepted by
            the `gh` CLI).

    Returns:
        A markdown string with the issue title, URL, description, and
        comments. Returns an empty string on failure.
    """
    data = run_gh_json(
        ["issue", "view", issue_id, "--json", "title,body,comments,url"],
        exit_on_error=False,
    )
    if not data:
        return f"# Issue #{issue_id}\n\n[Could not fetch issue details or comments]"

    parts = [f"# Issue #{issue_id}: {data.get('title', 'Unknown')}"]
    parts.append(f"URL: {data.get('url', '')}\n")

    parts.append("## Description")
    parts.append(data.get("body") or "*No description provided.*")

    comments = data.get("comments", [])
    if comments:
        parts.append("\n## Discussion Thread")
        for i, c in enumerate(comments, 1):
            author = c.get("author", {}).get("login", "Unknown")
            parts.append(f"\n### Comment {i} (@{author})")
            parts.append(c.get("body", ""))

    return "\n".join(parts)


def format_issues_context(issue_ids: list[str]) -> str:
    """Fetches and formats multiple GitHub issues into markdown.

    Args:
        issue_ids: List of issue IDs or numbers.

    Returns:
        Formatted markdown representation of all issues, separated by dividers.
    """
    outputs = []
    for issue_id in issue_ids:
        formatted = process_issue(issue_id)
        if formatted:
            outputs.append(formatted)

    return "\n\n---\n\n".join(outputs) if outputs else ""
