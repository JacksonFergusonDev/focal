import re
import subprocess
import sys

from focal.utils import run_gh_json


def filter_logs(raw_logs: str) -> str:
    """Processes raw GitHub Actions logs to remove noise."""
    processed_lines = []

    # Regex to match the GitHub Actions prefix:
    # e.g., 'Job Name\tStep Name\t2026-08-10T19:09:40.6560982Z '
    prefix_pattern = re.compile(r"^.*\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\s?")

    for line in raw_logs.splitlines():
        # Strip the prefix
        clean_line = prefix_pattern.sub("", line)
        processed_lines.append(clean_line)

    return "\n".join(processed_lines)


def main() -> None:
    """Executes the CLI script to fetch and format GitHub Actions CI failure logs.

    Retrieves the failed step logs and run metadata for a specified GitHub Actions
    run ID using the `gh` CLI, outputting a markdown-formatted block.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI commands fail to execute.
    """
    if len(sys.argv) != 2:
        sys.exit("Usage: python -m focal.gh_ci_fail <run_id>")

    run_id = sys.argv[1]

    log_res = subprocess.run(
        ["gh", "run", "view", run_id, "--log-failed"], capture_output=True, text=True
    )
    if log_res.returncode != 0:
        sys.exit(f"Error fetching logs: {log_res.stderr}")

    meta = run_gh_json(
        ["run", "view", run_id, "--json", "name,displayTitle"], exit_on_error=False
    )

    title = f"Run {run_id}"
    if meta:
        title = f"{meta.get('name') or 'CI'} - {meta.get('displayTitle') or ''}"

    print(f"# CI Failure Context: {title}\n")

    filtered_logs = filter_logs(log_res.stdout)
    print(f"```text\n[error logs]\n{filtered_logs.strip()}\n```")


if __name__ == "__main__":
    main()
