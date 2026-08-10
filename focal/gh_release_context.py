import json
import re
import subprocess
import sys


def main() -> None:
    """Executes the CLI script to fetch and format release context.

    Retrieves PRs merged into the repository since a specific date using the
    `gh` CLI and formats their metadata and bodies into a markdown document.
    Automatically excludes PRs authored by standard dependency bots.

    Raises:
        SystemExit: If the incorrect number of arguments is provided or if
            the `gh` CLI command fails.
    """
    if len(sys.argv) != 4:
        sys.exit(
            "Usage: python -m focal.gh_release_context <tag_date> <header_ref> <tag_ref>"
        )

    tag_date = sys.argv[1]
    header_ref = sys.argv[2]
    tag_ref = sys.argv[3]

    # Filter out standard dependency bots at the query level to preserve token bandwidth
    search_query = (
        f"is:pr is:merged base:main merged:>={tag_date} "
        "-author:app/renovate -author:dependabot -author:github-actions"
    )

    res = subprocess.run(
        [
            "gh",
            "pr",
            "list",
            "--search",
            search_query,
            "--json",
            "number,title,body,author,labels,url",
            "--limit",
            "100",
        ],
        capture_output=True,
        text=True,
    )

    if res.returncode != 0:
        sys.exit(f"Error fetching PRs: {res.stderr}")

    try:
        prs = json.loads(res.stdout)
    except json.JSONDecodeError:
        sys.exit("Error: Failed to parse JSON output from GitHub CLI.")

    parts = [f"# Release Context: {header_ref} to HEAD\n"]

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
        git_log_res = subprocess.run(
            [
                "git",
                "log",
                f"{tag_ref}..HEAD",
                "--no-merges",
                "--format=* `%h` - %s (@%an)",
            ],
            capture_output=True,
            text=True,
        )

        if git_log_res.returncode == 0 and git_log_res.stdout.strip():
            bot_authors = [
                "dependabot[bot]",
                "dependabot",
                "renovate[bot]",
                "renovate",
                "github-actions[bot]",
                "github-actions",
            ]
            commits = []
            for line in git_log_res.stdout.splitlines():
                line = line.strip()
                if not line:
                    continue
                is_bot = False
                for bot in bot_authors:
                    if f"(@{bot})" in line or f"(@{bot.replace('[bot]', '')})" in line:
                        is_bot = True
                        break
                if not is_bot:
                    commits.append(line)

            if commits:
                parts.append("## Raw Commits\n")
                parts.extend(commits)
                parts.append("\n")

    sys.stdout.write("\n".join(parts))


if __name__ == "__main__":
    main()
