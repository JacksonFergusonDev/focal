"""Bump the Focal version in every file that carries it, then commit, tag, and push.

The version lives in pyproject.toml and uv.lock (legacy Python package) and in
Cargo.toml and Cargo.lock (Rust binary). This follows the shared release script from
ci-cd-release-infrastructure: read-only pre-flight checks first, then a transactional
phase that rolls the repository back if any step fails.

    uv run scripts/release.py patch --dry-run   # validate and preview only
    uv run scripts/release.py minor --no-push   # commit and tag locally
    uv run scripts/release.py --check           # verify the files agree
"""

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = ("pyproject.toml", "uv.lock", "Cargo.toml", "Cargo.lock")
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
# The first `version` key of the named table, which is the project's own version.
TABLE_VERSION = r'(?ms)^(\[{table}\]\n(?:(?!^\[).)*?^version = ")([^"]+)(")'
# The root package entry in Cargo.lock.
LOCK_VERSION = r'(?m)^(name = "focal"\nversion = ")([^"]+)(")'


def say(message: str) -> None:
    """Print a progress line."""
    print(f"[release] {message}", flush=True)


def die(message: str) -> None:
    """Print an error and exit."""
    print(f"[release] error: {message}", file=sys.stderr)
    sys.exit(1)


def run(*cmd: str) -> str:
    """Run a command in the repository root and return its stripped stdout."""
    result = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=False)
    if result.returncode:
        raise RuntimeError(f"{' '.join(cmd)} failed:\n{result.stderr.strip()}")
    return result.stdout.strip()


def pattern(file: str) -> str:
    """The version pattern for one of the files the script edits directly."""
    return {
        "pyproject.toml": TABLE_VERSION.format(table="project"),
        "Cargo.toml": TABLE_VERSION.format(table="package"),
        "Cargo.lock": LOCK_VERSION,
    }[file]


def read_version(file: str) -> str:
    """Read the project version from pyproject.toml, Cargo.toml, or Cargo.lock."""
    match = re.search(pattern(file), (ROOT / file).read_text())
    if not match:
        die(f"no project version found in {file}")
        raise AssertionError
    return match.group(2)


def write_version(file: str, version: str) -> None:
    """Replace the project version in one file, keeping everything else intact."""
    path = ROOT / file
    text, count = re.subn(
        pattern(file), rf"\g<1>{version}\g<3>", path.read_text(), count=1
    )
    if count != 1:
        raise RuntimeError(f"could not update the version in {file}")
    path.write_text(text)


def current_version() -> str:
    """The version all files agree on; exits if they disagree."""
    versions = {file: read_version(file) for file in FILES if file != "uv.lock"}
    if len(set(versions.values())) != 1:
        die(f"version mismatch: {versions}")
    return versions["Cargo.toml"]


def bump(version: str, part: str) -> str:
    """Return `version` with `part` (major, minor, or patch) incremented."""
    match = SEMVER.match(version)
    if not match:
        die(f"current version '{version}' is not MAJOR.MINOR.PATCH")
        raise AssertionError
    major, minor, patch = (int(n) for n in match.groups())
    return {
        "major": f"{major + 1}.0.0",
        "minor": f"{major}.{minor + 1}.0",
        "patch": f"{major}.{minor}.{patch + 1}",
    }[part]


def preflight(args: argparse.Namespace) -> tuple[str, str]:
    """Read-only checks; returns the current and the new version."""
    for tool in ("git", "uv", "cargo"):
        if not shutil.which(tool):
            die(f"required tool '{tool}' not found in PATH")
    branch = run("git", "rev-parse", "--abbrev-ref", "HEAD")
    if branch != args.branch:
        die(f"releases are cut from '{args.branch}' (currently on '{branch}')")
    if run("git", "status", "--porcelain"):
        die("the working tree has uncommitted or untracked changes")
    run("git", "fetch", args.remote, args.branch, "--tags", "--quiet")
    if run("git", "rev-parse", "HEAD") != run(
        "git", "rev-parse", f"{args.remote}/{args.branch}"
    ):
        die(f"'{args.branch}' does not match '{args.remote}/{args.branch}'")
    old = current_version()
    new = bump(old, args.part)
    tag = f"v{new}"
    if run("git", "tag", "-l", tag) or run(
        "git", "ls-remote", "--tags", args.remote, tag
    ):
        die(f"tag '{tag}' already exists")
    return old, new


def release(args: argparse.Namespace) -> None:
    """Bump, verify the lockfiles, commit, tag, and push, rolling back on failure."""
    old, new = preflight(args)
    tag = f"v{new}"
    say(f"{old} -> {new} (tag {tag})")
    if args.dry_run:
        say("pre-flight checks passed (dry run, nothing changed)")
        return
    start = run("git", "rev-parse", "HEAD")
    try:
        for file in ("pyproject.toml", "Cargo.toml", "Cargo.lock"):
            write_version(file, new)
        run("uv", "lock")
        run("uv", "lock", "--check")
        # Fails if Cargo.lock still needs changes, without touching the network.
        run("cargo", "metadata", "--locked", "--offline", "--format-version", "1")
        if current_version() != new:
            raise RuntimeError("files disagree after the bump")
        run("git", "add", *FILES)
        run("git", "commit", "-m", f"chore: bump version to {new}")
        run("git", "tag", "-a", tag, "-m", f"Bump version to {tag}")
        if args.no_push:
            say(f"created {tag} locally; push skipped")
        else:
            run("git", "push", args.remote, "HEAD", tag, "--atomic")
            say(f"released {tag}")
    except (KeyboardInterrupt, Exception) as error:
        say("rolling back")
        subprocess.run(["git", "tag", "-d", tag], cwd=ROOT, capture_output=True)
        subprocess.run(["git", "reset", "--hard", start], cwd=ROOT, check=False)
        die(str(error) or type(error).__name__)


def main() -> None:
    """Parse arguments and run the release or the consistency check."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("part", nargs="?", choices=["major", "minor", "patch"])
    parser.add_argument("--check", action="store_true", help="verify versions agree")
    parser.add_argument("--dry-run", action="store_true", help="validate only")
    parser.add_argument("--no-push", action="store_true", help="commit and tag only")
    parser.add_argument("--branch", default="main")
    parser.add_argument("--remote", default="origin")
    args = parser.parse_args()
    if args.check:
        print(current_version())
    elif args.part:
        release(args)
    else:
        parser.error("a version part (major, minor, patch) or --check is required")


if __name__ == "__main__":
    main()
