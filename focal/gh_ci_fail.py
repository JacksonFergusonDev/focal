import json
import subprocess
import sys


def main() -> None:
    """Executes the CLI script to fetch and format GitHub Actions CI failure logs.

    Retrieves the failed step logs and run metadata for a specified GitHub Actions
    run ID using the `gh` CLI, outputting a markdown-formatted block.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI commands fail to execute.
    """
    if len(sys.argv) != 2:
        sys.exit("Usage: python -m ai_dev_tools.gh_ci_fail <run_id>")

    run_id = sys.argv[1]

    log_res = subprocess.run(
        ["gh", "run", "view", run_id, "--log-failed"], capture_output=True, text=True
    )
    if log_res.returncode != 0:
        sys.exit(f"Error fetching logs: {log_res.stderr}")

    meta_res = subprocess.run(
        ["gh", "run", "view", run_id, "--json", "name,displayTitle"],
        capture_output=True,
        text=True,
    )

    title = f"Run {run_id}"
    if meta_res.returncode == 0:
        meta = json.loads(meta_res.stdout)
        title = f"{meta.get('name', 'CI')} - {meta.get('displayTitle', '')}"

    print(f"# CI Failure Context: {title}\n")
    print(f"```text\n[error logs]\n{log_res.stdout.strip()}\n```")


if __name__ == "__main__":
    main()
