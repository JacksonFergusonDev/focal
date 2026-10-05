"""Explicit size limits for release-context payloads."""

import re

from focal.errors import print_warning


def limit_release_output(content: str, max_tokens: int = 24000) -> str:
    """Bound approximate tokens while retaining endpoint metadata and final counts."""
    if max_tokens == 0:
        return content
    max_chars = max_tokens * 4
    if len(content) <= max_chars:
        return content
    marker = "\n## Completeness Notes\n"
    body, separator, notes = content.rpartition(marker)
    if not separator:
        body, notes = content, ""
    # Keep the aggregate impact even when detailed PRs consume the budget.
    impact_start = body.find("\n## Release Impact\n")
    if impact_start >= 0:
        impact_end = body.find("\n## ", impact_start + 1)
        if impact_end < 0:
            impact_end = len(body)
        impact = body[impact_start:impact_end]
        if len(impact) > max_chars // 5:
            lines = impact.splitlines()
            impact = (
                "\n".join(lines[:40])
                + "\n...[diffstat paths abbreviated]\n"
                + "\n".join(lines[-2:])
                + "\n"
            )
        body = body[:impact_start] + body[impact_end:]
        notes = impact + marker + notes
    warning = (
        "\n\n## Output Limit\n\n"
        f"* WARNING: output exceeded the approximate {max_tokens:,}-token budget. "
        "Some release detail is omitted; this payload is incomplete. "
        "Rerun with a narrower range or increase --max-tokens; 0 disables this limit.\n"
    )
    footer = notes if impact_start >= 0 else marker + notes if separator else ""
    # Large numbers of diagnostics must not defeat the output bound either.
    footer_limit = max_chars // 3
    if len(footer) > footer_limit:
        footer = (
            footer[: footer_limit - 100].rsplit("\n", 1)[0]
            + "\n* Additional diagnostic detail omitted by output limit.\n"
        )
    allowance = max(0, max_chars - len(warning) - len(footer) - 150)
    clipped = body[:allowance].rsplit("\n", 1)[0]
    # Close an open fenced code block when a requested diff or description is cut.
    fences = re.findall(r"(?m)^(`{3,}|~{3,})[^\n]*$", clipped)
    if len(fences) % 2:
        clipped += "\n" + fences[-1]
    omitted = len(body) - len(clipped)
    warning += f"* {max(0, omitted):,} characters of release detail omitted from the end of the payload.\n"
    print_warning(
        f"Release context exceeded ~{max_tokens:,} tokens; detail omitted. Use --max-tokens 0 for full output."
    )
    return clipped + warning + footer
