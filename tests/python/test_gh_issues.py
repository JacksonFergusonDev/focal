from unittest.mock import patch

from focal.gh_issues import format_issues_context, process_issue


def test_process_issue_success():
    with patch("focal.gh_issues.run_gh_json") as mock_run_gh_json:
        mock_run_gh_json.return_value = {
            "title": "Bug in API",
            "url": "https://github.com/org/repo/issues/1",
            "body": "It crashes when I do X.",
            "comments": [
                {"author": {"login": "user1"}, "body": "I can confirm this."},
                {"author": {"login": "user2"}, "body": "Working on a fix."},
            ],
        }

        result = process_issue("1")

        assert "# Issue #1: Bug in API" in result
        assert "URL: https://github.com/org/repo/issues/1" in result
        assert "## Description\nIt crashes when I do X." in result
        assert "## Discussion Thread" in result
        assert "### Comment 1 (@user1)\nI can confirm this." in result
        assert "### Comment 2 (@user2)\nWorking on a fix." in result


def test_process_issue_not_found():
    with patch("focal.gh_issues.run_gh_json") as mock_run_gh_json:
        mock_run_gh_json.return_value = None

        result = process_issue("999")

        assert result == ""


def test_process_issue_no_description_or_comments():
    with patch("focal.gh_issues.run_gh_json") as mock_run_gh_json:
        mock_run_gh_json.return_value = {
            "title": "Empty issue",
            "url": "https://github.com/org/repo/issues/2",
            "body": None,
            "comments": [],
        }

        result = process_issue("2")

        assert "*No description provided.*" in result
        assert "## Discussion Thread" not in result


def test_format_issues_context_success():
    with patch("focal.gh_issues.process_issue") as mock_process_issue:
        mock_process_issue.side_effect = ["Output 1", "Output 2"]

        result = format_issues_context(["1", "2"])

        assert result == "Output 1\n\n---\n\nOutput 2"


def test_format_issues_context_empty():
    with patch("focal.gh_issues.process_issue", return_value=""):
        result = format_issues_context(["999"])
        assert result == ""
