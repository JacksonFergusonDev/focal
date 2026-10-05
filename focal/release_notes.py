"""Release-note instructions, previous-release style, and pinned media evidence."""

from pathlib import PurePosixPath
from typing import Any
from urllib.parse import quote

from focal.release_github import ReleaseGitHub
from focal.utils import run_git

MEDIA_EXTENSIONS = {
    ".apng",
    ".avif",
    ".gif",
    ".jpeg",
    ".jpg",
    ".mov",
    ".mp4",
    ".png",
    ".svg",
    ".webm",
    ".webp",
}


def release_notes_prompt(version: str, head: str) -> str:
    """Create the drafting request using the actual target commit."""
    return f"""# Release Notes Request

Write a title and Markdown release description for my GitHub release `{version}`, targeting commit `{head}`.
Use the previous release title and description included below as a reference for how I usually structure release notes, including tone, headings, and level of detail. Use it for style only: describe only changes supported by this release's evidence. If no previous release is available, use a concise, readable structure and tell me the style reference was unavailable. If the version is `vX.Y.Z`, keep that placeholder and ask me to confirm the version; do not invent a version.

If repository assets such as demos, screenshots, GIFs, or SVGs would help explain the release, include relevant links or Markdown embeds. Use the asset inventory below or resolve assets yourself at the release commit. Every repository asset URL must be pinned to the full release commit `{head}`; never use a branch, HEAD, or a movable tag, and do not reuse unpinned links from the previous release. Prefer a raw URL for image embeds and a file URL for video links. Do not assume an asset is useful from its filename alone; inspect it if you can.

If you cannot resolve or verify an asset, use a clearly labeled placeholder such as `[Demo](<ASSET_URL: path at {head}>)`. Do not invent URLs or claim a link was verified without checking it. The inventory confirms paths in the local Git tree, not remote availability or rendering. Include a separate checklist after the draft listing every proposed asset link or placeholder, its path and pinned commit, and what I should verify. Keep that checklist outside the release description so I can review it before publishing.

Return the proposed release title, the release description ready to paste into GitHub, and any verification items or gaps in the supplied evidence.
"""


def previous_release_context(
    repo: str | None,
    head: str,
    version: str,
    previous_tag: str | None = None,
    github: ReleaseGitHub | None = None,
) -> tuple[str, list[str]]:
    """Fetch the nearest published stable ancestor release, or an explicit reference."""
    github = github or ReleaseGitHub()
    notes: list[str] = []
    tag = previous_tag
    if not repo:
        return "*Previous release unavailable: GitHub repository lookup failed.*", [
            "WARNING: previous release style reference could not be fetched."
        ]
    if not tag:
        releases: list[dict[str, Any]] = []
        page = 1
        while True:
            result = github.request(
                ["api", f"repos/{repo}/releases?per_page=100&page={page}"],
            )
            if not isinstance(result, list):
                return "*Previous release unavailable: release listing failed.*", [
                    "WARNING: previous release discovery failed; style reference omitted."
                ]
            releases.extend(result)
            if len(result) < 100:
                break
            page += 1
        _, local_tags = run_git(["tag", "--merged", head])
        reachable = set(local_tags.splitlines())
        candidates = []
        missing = 0
        for release in releases:
            release_tag = release.get("tag_name")
            if (
                release.get("draft")
                or release.get("prerelease")
                or release_tag == version
            ):
                continue
            if release_tag not in reachable:
                missing += 1
                continue
            _, sha = run_git(["rev-parse", f"refs/tags/{release_tag}^{{commit}}"])
            if sha != head:
                candidates.append(release_tag)
        if missing:
            notes.append(
                f"{missing} published release tags were not reachable in the local checkout; they were excluded from automatic style selection."
            )
        if not candidates:
            return (
                "*No previous published stable release reachable before the target commit was found.*",
                notes,
            )
        args = ["describe", "--tags", "--abbrev=0", f"--candidates={len(candidates)}"]
        for candidate in candidates:
            args.extend(["--match", quote_glob(candidate)])
        _, tag = run_git([*args, head])
        notes.append(
            "Previous release style reference is the nearest locally reachable published stable release before the target commit, independent of the comparison base."
        )
    else:
        notes.append(
            f"Previous release style reference explicitly selected with --previous-release: {tag}; this does not change the Git range."
        )
    release = github.request(
        ["release", "view", tag, "--repo", repo, "--json", "name,body,tagName,url"],
    )
    if not isinstance(release, dict):
        return f"*Previous release `{tag}` unavailable: release lookup failed.*", [
            *notes,
            f"WARNING: could not fetch previous release {tag}; style reference omitted.",
        ]
    return "\n".join(
        [
            f"**Tag:** {release.get('tagName') or tag}",
            f"**Title:** {release.get('name') or tag}",
            f"**URL:** {release.get('url', '')}",
            "\n**Description (style reference only):**\n",
            release.get("body") or "*No description provided.*",
        ]
    ), notes


def quote_glob(tag: str) -> str:
    """Escape Git glob metacharacters so release tags match literally."""
    return "".join(f"[{char}]" if char in "*?[" else char for char in tag)


def release_asset_context(head: str, repo_url: str | None) -> str:
    """List tracked media at the immutable head with encoded, commit-pinned URLs."""
    _, tree = run_git(["ls-tree", "-r", "-z", head])
    rows = []
    for entry in tree.split("\0"):
        if not entry:
            continue
        metadata, path = entry.split("\t", 1)
        # Exclude symlinks and submodules: these do not establish media contents.
        if (
            not metadata.startswith("100")
            or PurePosixPath(path).suffix.lower() not in MEDIA_EXTENSIONS
        ):
            continue
        if repo_url:
            prefix = repo_url.rstrip("/")
            encoded = quote(path, safe="/")
            rows.append(
                f"* `{path}` — [file]({prefix}/blob/{head}/{encoded}), [raw]({prefix}/raw/{head}/{encoded})"
            )
        else:
            rows.append(
                f"* `{path}` — `<ASSET_URL: {path} at {head}>` (repository URL unavailable)"
            )
    return "\n".join(
        [
            f"Tracked image/video candidates at `{head}`. Paths exist in the local Git tree; remote URLs and rendering have not been verified. Choose only assets relevant to this release.",
            "",
            *(
                rows
                or [
                    "*No tracked image/video assets found at the release commit. Use a placeholder if a relevant demo needs to be supplied.*"
                ]
            ),
        ]
    )
