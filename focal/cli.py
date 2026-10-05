"""Unified internal Python CLI for focal subcommands.

Architectural Decision Record: Framework Selection (Click vs Argparse vs Typer)
--------------------------------------------------------------------------------------
Context:
    focal uses a fast-path Bash dispatcher (`bin/focal` and `libexec/*`) to ensure
    near-zero overhead for shell-heavy and interactive commands (e.g., fzf, rg, fd).
    Python is invoked only when AST parsing, DOM extraction, or complex Git/GitHub
    context processing is required.

Decision:
    We selected `click` as the internal CLI framework over `argparse` and `typer`:
    1. Startup Latency: `click` has zero third-party dependencies and adds only ~5-10ms
       to interpreter startup (totaling ~25-35ms). In contrast, `typer` relies on
       Pydantic and runtime type-reflection which can introduce 100-250ms of overhead.
    2. Maintainability & Ergonomics: `click` provides a clean, declarative decorator
       syntax (`@click.group()`, `@click.command()`), built-in parameter conversions,
       `click.Path` validation, and standardized help formatting with minimal boilerplate
       compared to standard library `argparse` subparsers.
    3. Stream & Pipe Ergonomics: Transparent standard input/output handling makes
       piped data flows clean and idiomatic.
    4. Testability: `click.testing.CliRunner` provides isolated, fast unit testing
       without requiring global process mocking or patching `sys.argv`.
"""

import click

from focal.gh_ci_fail import get_ci_failure_context
from focal.gh_issues import format_issues_context
from focal.gh_pr_diff import get_pr_diff_context
from focal.gh_release_context import get_release_context
from focal.notebook import MAX_OUTPUT_CHARS, notebook_to_llm_text
from focal.pdf import pdf_to_llm_text
from focal.wip_context import get_wip_context


@click.group(name="focal")
def cli() -> None:
    """Internal CLI for focal Python subcommands."""


@cli.command("ci-fail")
@click.argument("run_id")
def ci_fail_cmd(run_id: str) -> None:
    """Fetch and format GitHub Actions CI failure logs."""
    click.echo(get_ci_failure_context(run_id))


@cli.command("issues")
@click.argument("issue_ids", nargs=-1, required=True)
def issues_cmd(issue_ids: tuple[str, ...]) -> None:
    """Fetch and format GitHub issue threads."""
    output = format_issues_context(list(issue_ids))
    if output:
        click.echo(output)


@cli.command("pr-diff")
@click.argument("pr_id")
def pr_diff_cmd(pr_id: str) -> None:
    """Fetch and format GitHub Pull Request context and diff."""
    click.echo(get_pr_diff_context(pr_id))


@cli.command("release-context")
@click.option("--base", "base_ref", default=None)
@click.option(
    "--head",
    "head_ref",
    default=None,
    help="Explicit target ref; overrides version-tag selection.",
)
@click.option(
    "--level", type=click.Choice(["patch", "minor", "major"]), default="patch"
)
@click.option(
    "--bots", type=click.Choice(["summary", "include", "exclude"]), default="summary"
)
@click.option("--diff", "include_diff", is_flag=True)
@click.option("--path", "paths", multiple=True)
@click.option(
    "--version",
    default=None,
    help="Release version to draft; an existing local tag selects the target unless --head is given.",
)
@click.option(
    "--previous-release",
    default=None,
    help="GitHub release tag to use as the style reference.",
)
@click.option(
    "--request-timeout",
    type=click.FloatRange(min=0, min_open=True),
    default=10,
    show_default=True,
    help="Maximum seconds per GitHub request.",
)
@click.option(
    "--github-timeout",
    type=click.FloatRange(min=0, min_open=True),
    default=60,
    show_default=True,
    help="Total seconds available for GitHub enrichment.",
)
@click.option(
    "--max-tokens",
    type=click.IntRange(min=0),
    default=24000,
    show_default=True,
    help="Approximate output budget; 0 disables the limit.",
)
def release_context_cmd(
    base_ref: str | None,
    head_ref: str | None,
    level: str,
    bots: str,
    include_diff: bool,
    paths: tuple[str, ...],
    version: str | None,
    previous_release: str | None,
    request_timeout: float,
    github_timeout: float,
    max_tokens: int,
) -> None:
    """Collect Git release membership, PR intent, and net impact."""
    if 0 < max_tokens < 1000:
        raise click.UsageError("--max-tokens must be 0 or at least 1000")
    if paths and not include_diff:
        raise click.UsageError("--path requires --diff")
    click.echo(
        get_release_context(
            base_ref,
            head_ref,
            level,
            bots,
            include_diff,
            paths,
            version=version,
            previous_release=previous_release,
            request_timeout=request_timeout,
            github_timeout=github_timeout,
            max_tokens=max_tokens,
        )
    )


@cli.command("notebook")
@click.argument("path", type=click.Path(exists=True, dir_okay=False, readable=True))
@click.option(
    "--max-output",
    type=int,
    default=MAX_OUTPUT_CHARS,
    show_default=True,
    help="Max characters per cell output before truncation.",
)
def notebook_cmd(path: str, max_output: int) -> None:
    """Extract and format Jupyter notebook (.ipynb) files."""
    click.echo(notebook_to_llm_text(path, max_output_chars=max_output), nl=False)


@cli.command("pdf")
@click.argument("path", type=click.Path(exists=True, dir_okay=False, readable=True))
def pdf_cmd(path: str) -> None:
    """Extract and format PDF documents."""
    click.echo(pdf_to_llm_text(path))


@cli.command("wip-context")
@click.argument("target_branch", required=False, default=None)
def wip_context_cmd(target_branch: str | None) -> None:
    """Generate work-in-progress branch topology and diff context."""
    click.echo(get_wip_context(target_branch), nl=False)


def main() -> None:
    """CLI entrypoint."""
    cli()


if __name__ == "__main__":
    main()
