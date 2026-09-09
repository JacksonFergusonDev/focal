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
from focal.notebook import notebook_to_llm_text
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
@click.argument("tag_date")
@click.argument("header_ref")
@click.argument("tag_ref")
@click.argument("head_ref", default="HEAD", required=False)
@click.argument("head_date", default=None, required=False)
def release_context_cmd(
    tag_date: str,
    header_ref: str,
    tag_ref: str,
    head_ref: str,
    head_date: str | None,
) -> None:
    """Collect merged pull requests and git commit histories for releases."""
    output = get_release_context(
        tag_date, header_ref, tag_ref, head_ref=head_ref, head_date=head_date
    )
    click.echo(output)


@cli.command("notebook")
@click.argument("path", type=click.Path(exists=True, dir_okay=False, readable=True))
def notebook_cmd(path: str) -> None:
    """Extract and format Jupyter notebook (.ipynb) files."""
    click.echo(notebook_to_llm_text(path), nl=False)


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
