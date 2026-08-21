"""Extracts and formats failed GitHub Actions CI logs into markdown for LLM consumption."""

import re
import subprocess
import sys

from focal.errors import die
from focal.utils import run_gh_json


def filter_logs(raw_logs: str) -> str:
    """Processes raw GitHub Actions logs to remove noise.

    Strips log prefixes, ANSI escape sequences, and successful output groups,
    retaining context around lines containing errors.

    Args:
        raw_logs: The raw text logs from a GitHub Actions run.

    Returns:
        The filtered, markdown-ready log text focused on error occurrences.
    """
    processed_lines = []

    # Regex to match the GitHub Actions prefix:
    # e.g., 'Job Name\tStep Name\t2026-08-10T19:09:40.6560982Z '
    prefix_pattern = re.compile(r"^.*\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\s?")

    # Regex to match ANSI escape codes
    ansi_escape = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")

    in_group = False
    group_name = ""
    group_buffer = []
    group_has_error = False

    ignored_substrings = [
        "Download action repository",
        "Secret source: Actions",
        "Prepare workflow directory",
        "Getting action download info",
        "Complete job name:",
    ]

    for line in raw_logs.splitlines():
        # Strip the prefix and ANSI codes
        clean_line = prefix_pattern.sub("", line)
        clean_line = ansi_escape.sub("", clean_line)

        # Skip known boilerplate lines if they are not in a failed group context
        # (Actually, it's safe to just skip them entirely to reduce noise)
        if any(ignored in clean_line for ignored in ignored_substrings):
            continue

        if clean_line.startswith("##[group]"):
            in_group = True
            group_name = clean_line[len("##[group]") :]
            group_buffer = [clean_line]
            group_has_error = False
        elif clean_line.startswith("##[endgroup]"):
            if in_group:
                group_buffer.append(clean_line)
                if group_has_error:
                    processed_lines.extend(group_buffer)
                else:
                    processed_lines.append(
                        f"> [Group completed successfully: {group_name}]"
                    )
                in_group = False
                group_buffer = []
            else:
                processed_lines.append(clean_line)
        elif in_group:
            group_buffer.append(clean_line)
            if "##[error]" in clean_line:
                group_has_error = True
        else:
            processed_lines.append(clean_line)

    # If a group didn't close properly, append its buffer
    if in_group:
        processed_lines.extend(group_buffer)

    # Limit output to a window around errors to avoid massive dumps
    error_indices = [i for i, line in enumerate(processed_lines) if "##[error]" in line]
    if not error_indices:
        return "\n".join(processed_lines)

    keep_indices = set()
    for idx in error_indices:
        # Keep 15 lines before and 10 lines after the error
        for j in range(max(0, idx - 15), min(len(processed_lines), idx + 11)):
            keep_indices.add(j)

    final_lines = []
    last_idx = -2
    for idx in sorted(keep_indices):
        if idx > last_idx + 1 and last_idx != -2:
            final_lines.append("...")
        final_lines.append(processed_lines[idx])
        last_idx = idx

    return "\n".join(final_lines)


def get_ci_failure_context(run_id: str) -> str:
    """Fetches and formats GitHub Actions CI failure logs for a specific run ID.

    Args:
        run_id: The GitHub Actions run ID.

    Returns:
        A markdown-formatted string with run metadata and filtered error logs.

    Raises:
        SystemExit: If fetching logs fails.
    """
    log_res = subprocess.run(
        ["gh", "run", "view", run_id, "--log-failed"], capture_output=True, text=True
    )
    if log_res.returncode != 0:
        die(
            f"error fetching logs: {log_res.stderr.strip()}",
            hint="verify that the run ID exists and you have access",
        )

    meta = run_gh_json(
        ["run", "view", run_id, "--json", "name,displayTitle"], exit_on_error=False
    )

    title = f"Run {run_id}"
    if meta:
        title = f"{meta.get('name') or 'CI'} - {meta.get('displayTitle') or ''}"

    filtered_logs = filter_logs(log_res.stdout)
    return f"# CI Failure Context: {title}\n\n```text\n[error logs]\n{filtered_logs.strip()}\n```"


def main() -> None:
    """Executes the CLI script to fetch and format GitHub Actions CI failure logs.

    Retrieves the failed step logs and run metadata for a specified GitHub Actions
    run ID using the `gh` CLI, outputting a markdown-formatted block.

    Raises:
        SystemExit: If the incorrect number of arguments is provided, or if the
            `gh` CLI commands fail to execute.
    """
    if len(sys.argv) != 2:
        die(
            "missing run_id argument", hint="usage: python -m focal.gh_ci_fail <run_id>"
        )

    run_id = sys.argv[1]
    print(get_ci_failure_context(run_id))


if __name__ == "__main__":
    main()
