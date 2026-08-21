from unittest.mock import patch

from click.testing import CliRunner

from focal.cli import cli


def test_cli_help():
    runner = CliRunner()
    result = runner.invoke(cli, ["--help"])
    assert result.exit_code == 0
    assert "Internal CLI for focal Python subcommands." in result.output
    assert "ci-fail" in result.output
    assert "issues" in result.output
    assert "pr-diff" in result.output
    assert "release-context" in result.output
    assert "notebook" in result.output
    assert "pdf" in result.output
    assert "web" in result.output
    assert "wip-context" in result.output


def test_cli_ci_fail():
    runner = CliRunner()
    with patch("focal.cli.get_ci_failure_context") as mock_fn:
        mock_fn.return_value = "# CI Failure Context: Run 123"
        result = runner.invoke(cli, ["ci-fail", "123"])
        assert result.exit_code == 0
        assert "# CI Failure Context: Run 123" in result.output
        mock_fn.assert_called_once_with("123")


def test_cli_issues():
    runner = CliRunner()
    with patch("focal.cli.format_issues_context") as mock_fn:
        mock_fn.return_value = "# Issue #1\n\n---\n\n# Issue #2"
        result = runner.invoke(cli, ["issues", "1", "2"])
        assert result.exit_code == 0
        assert "# Issue #1" in result.output
        mock_fn.assert_called_once_with(["1", "2"])


def test_cli_pr_diff():
    runner = CliRunner()
    with patch("focal.cli.get_pr_diff_context") as mock_fn:
        mock_fn.return_value = "# PR #42: Feature"
        result = runner.invoke(cli, ["pr-diff", "42"])
        assert result.exit_code == 0
        assert "# PR #42: Feature" in result.output
        mock_fn.assert_called_once_with("42")


def test_cli_release_context():
    runner = CliRunner()
    with patch("focal.cli.get_release_context") as mock_fn:
        mock_fn.return_value = "# Release Context"
        result = runner.invoke(
            cli,
            ["release-context", "2026-01-01", "v1.0.0", "v0.9.0", "HEAD", "2026-02-01"],
        )
        assert result.exit_code == 0
        assert "# Release Context" in result.output
        mock_fn.assert_called_once_with(
            "2026-01-01", "v1.0.0", "v0.9.0", head_ref="HEAD", head_date="2026-02-01"
        )


def test_cli_notebook(tmp_path):
    nb_file = tmp_path / "test.ipynb"
    nb_file.write_text('{"cells": []}')

    runner = CliRunner()
    with patch("focal.cli.notebook_to_llm_text") as mock_fn:
        mock_fn.return_value = "# Notebook: parsed"
        result = runner.invoke(cli, ["notebook", str(nb_file)])
        assert result.exit_code == 0
        assert "# Notebook: parsed" in result.output
        mock_fn.assert_called_once_with(str(nb_file))


def test_cli_pdf(tmp_path):
    pdf_file = tmp_path / "doc.pdf"
    pdf_file.write_bytes(b"%PDF-1.4...")

    runner = CliRunner()
    with patch("focal.cli.pdf_to_llm_text") as mock_fn:
        mock_fn.return_value = "PDF text content"
        result = runner.invoke(cli, ["pdf", str(pdf_file)])
        assert result.exit_code == 0
        assert "PDF text content" in result.output
        mock_fn.assert_called_once_with(str(pdf_file))


def test_cli_web_with_url():
    runner = CliRunner()
    with patch("focal.cli.get_web_context") as mock_fn:
        mock_fn.return_value = "# Source: https://example.com"
        result = runner.invoke(cli, ["web", "https://example.com"])
        assert result.exit_code == 0
        assert "# Source: https://example.com" in result.output
        mock_fn.assert_called_once_with(url="https://example.com")


def test_cli_web_with_stdin():
    runner = CliRunner()
    with patch("focal.cli.get_web_context") as mock_fn:
        mock_fn.return_value = "# Source: Piped DOM/Clipboard\n\nHello World"
        result = runner.invoke(cli, ["web"], input="<p>Hello World</p>")
        assert result.exit_code == 0
        assert "Hello World" in result.output
        mock_fn.assert_called_once_with(html_content="<p>Hello World</p>")


def test_cli_wip_context():
    runner = CliRunner()
    with patch("focal.cli.get_wip_context") as mock_fn:
        mock_fn.return_value = "# WIP Branch Context"
        result = runner.invoke(cli, ["wip-context", "main"])
        assert result.exit_code == 0
        assert "# WIP Branch Context" in result.output
        mock_fn.assert_called_once_with("main")
