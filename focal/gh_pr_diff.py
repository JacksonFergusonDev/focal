import subprocess
import sys

from focal.utils import run_gh_json


def main() -> None:
    """Executes the CLI script to fetch and format a GitHub Pull Request context.

    Retrieves the metadata, intent (body), and code diff for a specified GitHub
    PR using the `gh` CLI, outputting a markdown-formatted document.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI commands fail to execute.
    """
    if len(sys.argv) != 2:
        sys.exit("Usage: python -m ai_dev_tools.gh_pr_diff <pr_id>")

    pr_id = sys.argv[1]

    data = run_gh_json(["pr", "view", pr_id, "--json", "title,body,url"])

    diff_res = subprocess.run(
        ["gh", "pr", "diff", pr_id], capture_output=True, text=True
    )
    if diff_res.returncode != 0:
        sys.exit(f"Error fetching PR diff: {diff_res.stderr}")

    parts = [f"# PR #{pr_id}: {data.get('title', 'Unknown')}"]
    parts.append(f"URL: {data.get('url', '')}\n")

    parts.append("## Intent / Description")
    parts.append(data.get("body") or "*No description provided.*")

    parts.append("\n## Diff")
    parts.append(f"```diff\n{diff_res.stdout.strip()}\n```")

    print("\n".join(parts))


if __name__ == "__main__":
    main()
