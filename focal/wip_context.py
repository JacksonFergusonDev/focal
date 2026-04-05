import subprocess
import sys
from pathlib import Path

# Conservative character limit for the diff section (~8k tokens) to maintain high LLM attention
MAX_DIFF_CHARS = 30000

# Expanded noise extensions covering data formats, serialized objects, media, and archives
NOISE_EXTENSIONS = {
    # Data & Serialization
    ".csv",
    ".tsv",
    ".json",
    ".xml",
    ".parquet",
    ".pkl",
    ".sqlite",
    ".db",
    ".npy",
    ".npz",
    ".h5",
    ".hdf5",
    ".fits",
    ".data",
    ".nc",
    # Media & Assets
    ".svg",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".webp",
    ".pdf",
    ".mp4",
    ".webm",
    ".mov",
    ".avi",
    ".mkv",
    ".mp3",
    ".wav",
    # Frontend build artifacts
    ".min.js",
    ".min.css",
    ".map",
    # Archives & Compiled
    ".zip",
    ".tar",
    ".gz",
    ".xz",
    ".bz2",
    ".whl",
    ".pyc",
    ".bin",
    ".exe",
    ".so",
    ".dylib",
    ".dll",
}

NOISE_FILES = {
    "poetry.lock",
    "uv.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
}


def run_git(args: list[str], check: bool = True) -> tuple[int, str]:
    """Executes a Git command and returns its exit code and standard output.

    Args:
        args (list[str]): The Git subcommand and its arguments.
        check (bool): If True, exits the program if the Git command fails.

    Returns:
        tuple[int, str]: The return code and the stripped standard output.

    Raises:
        SystemExit: If the Git command fails and `check` is True.
    """
    res = subprocess.run(
        ["git", *args],
        capture_output=True,
        text=True,
    )
    if check and res.returncode != 0:
        sys.exit(
            f"Git command failed: git {' '.join(args)}\nError: {res.stderr.strip()}"
        )

    return res.returncode, res.stdout.strip()


def resolve_base_branch(target: str | None) -> str:
    """Determines the target base branch to compare HEAD against.

    If no target is provided, sequentially attempts to verify the existence
    of 'main', 'master', and 'develop'.

    Args:
        target (str | None): A user-specified branch name, or None.

    Returns:
        str: The resolved branch name.

    Raises:
        SystemExit: If no branch is provided and standard defaults are not found.
    """
    if target:
        code, _ = run_git(["rev-parse", "--verify", target], check=False)
        if code != 0:
            sys.exit(f"Error: Specified base branch '{target}' does not exist.")
        return target

    fallbacks = ["main", "master", "develop"]
    for branch in fallbacks:
        code, _ = run_git(["rev-parse", "--verify", branch], check=False)
        if code == 0:
            return branch

    sys.exit(
        "Error: Could not automatically detect a base branch (tried main, master, develop).\n"
        "Please specify it explicitly: focal wip-context <branch>"
    )


def is_priority(filepath: str) -> bool:
    """Determines if a file is a high-priority context file.

    High-priority files are those situated at the root of the repository
    or within the core `.github/` configuration directory.

    Args:
        filepath (str): The relative path to the file in the repository.

    Returns:
        bool: True if the file is priority, False otherwise.
    """
    if "/" not in filepath:
        return True
    return bool(filepath.startswith(".github/"))


def is_noise(filepath: str) -> bool:
    """Determines if a file should be excluded from the text diff.

    Args:
        filepath (str): The relative path to the file.

    Returns:
        bool: True if the file matches known noise patterns, False otherwise.
    """
    path = Path(filepath)
    if path.name in NOISE_FILES:
        return True
    return path.suffix in NOISE_EXTENSIONS


def get_diff_for_files(
    base: str, files: list[str], chars_remaining: int
) -> tuple[list[str], int, int]:
    """Fetches Git diffs for a list of files up to a character limit.

    Args:
        base (str): The base commit hash to compare against.
        files (list[str]): The list of file paths to diff.
        chars_remaining (int): The maximum number of characters allowed for the output.

    Returns:
        tuple[list[str], int, int]: The list of formatted diff strings,
            the updated remaining character count, and the number of omitted files.
    """
    diff_blocks = []
    omitted_count = 0

    for file in files:
        if chars_remaining <= 0:
            omitted_count += 1
            continue

        _, diff_text = run_git(["diff", f"{base}..HEAD", "--", file])
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
            omitted_count += 1
            chars_remaining = 0  # Force remaining files in the loop to be skipped
        else:
            diff_blocks.append(block)
            chars_remaining -= len(block)

    return diff_blocks, chars_remaining, omitted_count


def main() -> None:
    """Executes the CLI script to gather context from a WIP branch.

    Identifies the merge base, extracts the commit topology, builds a macroscopic
    file stat map, and intelligently appends file diffs based on priority and size.
    Outputs a markdown-formatted document to standard output.
    """
    # Verify we are in a valid Git repository before doing anything
    run_git(["rev-parse", "--is-inside-work-tree"])

    target_arg = sys.argv[1] if len(sys.argv) > 1 else None
    target_branch = resolve_base_branch(target_arg)

    # Resolve references
    _, base_commit = run_git(["merge-base", target_branch, "HEAD"])
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
        sys.exit(f"No divergent commits found between '{target_branch}' and HEAD.")
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
        f"**Base:** `{target_branch}` ({short_base}) | **HEAD:** ({head_commit})",
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
        parts.append(
            f"*Note: Diffs for {len(omitted_files)} noise/binary files were explicitly excluded.*"
        )

    if diff_output:
        parts.extend(diff_output)
    else:
        parts.append("*No text diffs available or all changes were in noise files.*")

    if total_omitted > 0:
        parts.append(
            f"\n*...diffs for {total_omitted} remaining files omitted (context limit reached).*"
        )

    sys.stdout.write("\n".join(parts) + "\n")


if __name__ == "__main__":
    main()
