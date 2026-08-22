"""Gathers branch topology, commit history, file stats, and diffs for work-in-progress Git branches."""

import json
import subprocess
from pathlib import Path

from focal.errors import die
from focal.utils import resolve_base_branch, run_git

# Conservative character limit for the diff section (~8k tokens) to maintain high LLM attention
MAX_DIFF_CHARS = 30000


def _load_noise_config() -> tuple[set[str], set[str]]:
    """Loads noise extensions and noise files from shared noise.json."""
    noise_path = Path(__file__).resolve().parent.parent / "lib" / "noise.json"
    if noise_path.is_file():
        try:
            data = json.loads(noise_path.read_text(encoding="utf-8"))
            exts = {f".{ext.lstrip('.')}" for ext in data.get("extensions", [])}
            files = set(data.get("files", []))
            return exts, files
        except Exception:
            pass
    return set(), set()


NOISE_EXTENSIONS, NOISE_FILES = _load_noise_config()


def is_priority(filepath: str) -> bool:
    """Determines if a file is a high-priority context file.

    High-priority files are those situated at the root of the repository
    or within the core `.github/` configuration directory.

    Args:
        filepath: The relative path to the file in the repository.

    Returns:
        True if the file is priority, False otherwise.
    """
    if "/" not in filepath:
        return True
    return bool(filepath.startswith(".github/"))


def is_noise(filepath: str) -> bool:
    """Determines if a file should be excluded from the text diff.

    Args:
        filepath: The relative path to the file.

    Returns:
        True if the file matches known noise patterns, False otherwise.
    """
    path = Path(filepath)
    if path.name in NOISE_FILES:
        return True
    return any(path.name.endswith(ext) for ext in NOISE_EXTENSIONS)


def get_diff_for_files(
    base: str, files: list[str], chars_remaining: int
) -> tuple[list[str], int, int]:
    """Fetches Git diffs for a list of files up to a character limit.

    Args:
        base: The base commit hash to compare against.
        files: The list of file paths to diff.
        chars_remaining: The maximum number of characters allowed for the output.

    Returns:
        The list of formatted diff strings, the updated remaining character count,
        and the number of omitted files.
    """
    diff_blocks = []
    omitted_count = 0

    for file in files:
        if chars_remaining <= 0:
            omitted_count += 1
            continue

        _, diff_text = run_git(["diff", "-M", f"{base}..HEAD", "--", file])
        if not diff_text:
            continue

        # Catch-all for binaries not caught by NOISE_EXTENSIONS
        if diff_text.startswith("Binary files") and diff_text.strip().endswith(
            "differ"
        ):
            continue

        # Strip 'index <hash>..<hash> <mode>' lines to save tokens
        cleaned_lines = [
            line for line in diff_text.split("\n") if not line.startswith("index ")
        ]
        cleaned_diff = "\n".join(cleaned_lines)

        block = f"### `{file}`\n```diff\n{cleaned_diff}\n```"

        if len(block) > chars_remaining:
            # Truncate the block to fit the remaining budget, ensuring we close the markdown fence.
            # We allocate a ~60 char buffer for the truncation warning and closing ticks.
            if chars_remaining > 100:
                sliced_diff = block[: chars_remaining - 60]
                sliced_diff += "\n...[diff truncated: context limit reached]\n```"
                diff_blocks.append(sliced_diff)

            omitted_count += 1
            chars_remaining = (
                0  # Deplete the budget to trigger skipping for subsequent files
            )
        else:
            diff_blocks.append(block)
            chars_remaining -= len(block)

    return diff_blocks, chars_remaining, omitted_count


def get_wip_context(target_branch: str | None = None) -> str:
    """Gathers branch topology, commit history, file stats, and diffs for a WIP branch.

    Args:
        target_branch: Optional base branch to compare against.

    Returns:
        A markdown-formatted string with WIP branch context.

    Raises:
        SystemExit: If not inside a Git worktree or if branch resolution fails.
    """
    # Verify we are in a valid Git repository before doing anything
    run_git(["rev-parse", "--is-inside-work-tree"])

    resolved_branch = resolve_base_branch(target_branch)

    # Resolve references
    _, base_commit = run_git(["merge-base", resolved_branch, "HEAD"])
    _, head_commit = run_git(["rev-parse", "--short", "HEAD"])
    short_base = base_commit[:7]

    # Layer 0: Working Tree State
    _, status = run_git(["status", "--porcelain"])

    # Layer 1: Topology (now explicitly tracking file associations)
    _, topology = run_git(
        [
            "log",
            "--name-status",
            "--pretty=format:%n[%h] %s (%cr)",
            f"{base_commit}..HEAD",
        ]
    )
    if not topology:
        die(
            f"no divergent commits found between '{resolved_branch}' and HEAD",
            hint="create commits on this branch or specify a different base branch: focal wip-context <branch>",
        )
    topology = topology.strip()

    # Layer 2: Macroscopic Map
    _, diff_stat = run_git(["diff", "--stat", f"{base_commit}..HEAD"])

    # Layer 3: Microscopic Diffs
    _, changed_files_raw = run_git(["diff", "--name-only", f"{base_commit}..HEAD"])
    changed_files = [f for f in changed_files_raw.split("\n") if f]

    priority_files = []
    standard_files = []
    omitted_files = []

    for f in changed_files:
        if is_noise(f):
            omitted_files.append(f)
        elif is_priority(f):
            priority_files.append(f)
        else:
            standard_files.append(f)

    diff_output = []
    chars_left = MAX_DIFF_CHARS

    p_blocks, chars_left, p_omitted = get_diff_for_files(
        base_commit, priority_files, chars_left
    )
    diff_output.extend(p_blocks)

    s_blocks, chars_left, s_omitted = get_diff_for_files(
        base_commit, standard_files, chars_left
    )
    diff_output.extend(s_blocks)

    total_omitted = p_omitted + s_omitted

    # Document Assembly
    parts = [
        "# WIP Branch Context",
        f"**Base:** `{resolved_branch}` ({short_base}) | **HEAD:** ({head_commit})",
    ]

    if status:
        parts.extend(["\n## 0. Uncommitted Changes", "```text", status, "```"])

    parts.extend(
        [
            "\n## 1. Branch Topology",
            "```text",
            topology,
            "```",
            "\n## 2. Macroscopic Map",
            "```text",
            diff_stat,
            "```",
            "\n## 3. Microscopic Diffs",
        ]
    )

    if omitted_files:
        parts.append("\n## 4. Omitted Files Metadata")
        _, repo_root = run_git(["rev-parse", "--show-toplevel"])
        repo_root_path = Path(repo_root)
        for f in omitted_files:
            try:
                full_path = repo_root_path / f
                size_bytes = full_path.stat().st_size
                if size_bytes < 1024:
                    size_str = f"{size_bytes}B"
                elif size_bytes < 1024 * 1024:
                    size_str = f"{size_bytes / 1024:.1f}K"
                else:
                    size_str = f"{size_bytes / (1024 * 1024):.1f}M"

                res = subprocess.run(
                    ["file", "-b", str(full_path)], capture_output=True, text=True
                )
                meta = res.stdout.strip() if res.returncode == 0 else "Unknown"

                parts.append(f"- `{f}` (Size: {size_str})\n  - Type: {meta}")
            except Exception:
                parts.append(f"- `{f}` (Deleted or inaccessible)")

    if diff_output:
        parts.extend(diff_output)
    else:
        parts.append("*No text diffs available or all changes were in noise files.*")

    if total_omitted > 0:
        parts.append(
            f"\n*...diffs for {total_omitted} remaining files omitted (context limit reached).*"
        )

    return "\n".join(parts) + "\n"
