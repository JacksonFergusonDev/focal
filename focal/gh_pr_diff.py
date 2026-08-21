"""Fetches and formats GitHub Pull Request descriptions and diffs into markdown."""

from focal.errors import die
from focal.utils import run_gh, run_gh_json


def get_pr_diff_context(pr_id: str) -> str:
    """Fetches and formats a GitHub Pull Request context.

    Args:
        pr_id: The pull request number or identifier.

    Returns:
        Formatted markdown representation of the PR metadata, description, and diff.

    Raises:
        SystemExit: If the PR diff cannot be fetched.
    """
    data = run_gh_json(["pr", "view", pr_id, "--json", "title,body,url"])

    code, diff_out = run_gh(["pr", "diff", pr_id], exit_on_error=False)
    if code != 0:
        die(
            f"error fetching PR diff: {diff_out}",
            hint="verify that the PR exists and you have access",
        )

    parts = [f"# PR #{pr_id}: {data.get('title', 'Unknown')}"]
    parts.append(f"URL: {data.get('url', '')}\n")

    parts.append("## Intent / Description")
    parts.append(data.get("body") or "*No description provided.*")

    parts.append("\n## Diff")
    parts.append(f"```diff\n{diff_out.strip()}\n```")

    return "\n".join(parts)
