"""Fetches and formats GitHub Pull Request descriptions and diffs into markdown."""

import subprocess
import sys

from focal.errors import die
from focal.utils import run_gh_json


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

    diff_res = subprocess.run(
        ["gh", "pr", "diff", pr_id], capture_output=True, text=True
    )
    if diff_res.returncode != 0:
        die(
            f"error fetching PR diff: {diff_res.stderr.strip()}",
            hint="verify that the PR exists and you have access",
        )

    parts = [f"# PR #{pr_id}: {data.get('title', 'Unknown')}"]
    parts.append(f"URL: {data.get('url', '')}\n")

    parts.append("## Intent / Description")
    parts.append(data.get("body") or "*No description provided.*")

    parts.append("\n## Diff")
    parts.append(f"```diff\n{diff_res.stdout.strip()}\n```")

    return "\n".join(parts)


def main() -> None:
    """Executes the CLI script to fetch and format a GitHub Pull Request context.

    Retrieves the metadata, intent (body), and code diff for a specified GitHub
    PR using the `gh` CLI, outputting a markdown-formatted document.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI commands fail to execute.
    """
    if len(sys.argv) != 2:
        die("missing PR ID argument", hint="usage: python -m focal.gh_pr_diff <pr_id>")

    pr_id = sys.argv[1]
    print(get_pr_diff_context(pr_id))


if __name__ == "__main__":
    main()
