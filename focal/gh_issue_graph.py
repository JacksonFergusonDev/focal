import json
import subprocess
import sys


def main() -> None:
    """Executes the CLI script to fetch and format a GitHub issue thread.

    Retrieves the title, body, and comment thread for a specified GitHub issue
    using the `gh` CLI, outputting a sequentially formatted markdown document.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI command fails to execute.
    """
    if len(sys.argv) != 2:
        sys.exit("Usage: python -m ai_dev_tools.gh_issue_graph <issue_id>")

    issue_id = sys.argv[1]

    res = subprocess.run(
        ["gh", "issue", "view", issue_id, "--json", "title,body,comments,url"],
        capture_output=True,
        text=True,
    )
    if res.returncode != 0:
        sys.exit(f"Error fetching issue: {res.stderr}")

    data = json.loads(res.stdout)

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

    print("\n".join(parts))


if __name__ == "__main__":
    main()
