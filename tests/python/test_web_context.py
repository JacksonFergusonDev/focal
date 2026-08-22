import urllib.error
from unittest.mock import MagicMock, patch

import pytest

from focal.web_context import fetch_url, get_web_context, parse_html_to_md


def test_parse_html_to_md():
    html_input = """
    <html>
        <head>
            <title>Test Page</title>
            <script>console.log("noisy")</script>
            <style>body { color: red; }</style>
        </head>
        <body>
            <header>Header</header>
            <nav>Navigation</nav>
            <main>
                <h1>Main Content</h1>
                <p>This is a paragraph.</p>
                <button>Click me</button>
            </main>
            <footer>Footer</footer>
        </body>
    </html>
    """

    md_output = parse_html_to_md(html_input, "https://example.com")

    assert "# Source: https://example.com" in md_output
    assert "# Main Content" in md_output
    assert "This is a paragraph." in md_output

    # Check that noise tags are removed
    assert "noisy" not in md_output
    assert "color: red" not in md_output
    assert "Header" not in md_output
    assert "Navigation" not in md_output
    assert "Footer" not in md_output
    assert "Click me" not in md_output


def test_fetch_url_success():
    with patch("focal.web_context.urllib.request.urlopen") as mock_urlopen:
        mock_response = MagicMock()
        mock_response.read.return_value = b"<html>Test</html>"
        mock_response.headers.get_content_charset.return_value = "utf-8"
        mock_urlopen.return_value.__enter__.return_value = mock_response

        result = fetch_url("https://example.com")

        assert result == "<html>Test</html>"


def test_fetch_url_error():
    with patch("focal.web_context.urllib.request.urlopen") as mock_urlopen:
        mock_urlopen.side_effect = urllib.error.URLError("Connection refused")

        with pytest.raises(SystemExit) as exc_info:
            fetch_url("https://example.com")

        assert "Error fetching https://example.com" in str(exc_info.value)


def test_get_web_context_with_url():
    with patch("focal.web_context.fetch_url") as mock_fetch:
        mock_fetch.return_value = "<h1>Fetched</h1>"

        output = get_web_context(url="https://example.com")
        assert "# Source: https://example.com" in output
        assert "# Fetched" in output


def test_get_web_context_with_stdin_html():
    output = get_web_context(html_content="<h1>Piped</h1>")
    assert "# Source: Piped DOM/Clipboard" in output
    assert "# Piped" in output


def test_get_web_context_with_empty_html():
    with pytest.raises(SystemExit) as exc_info:
        get_web_context(html_content="   ")

    assert "received empty piped input" in str(exc_info.value)


def test_get_web_context_missing_args():
    with pytest.raises(SystemExit) as exc_info:
        get_web_context()

    assert "missing URL or HTML input" in str(exc_info.value)


def test_parse_html_to_md_truncation():
    large_html = "<p>" + "A" * 60000 + "</p>"
    md_output = parse_html_to_md(large_html, "https://example.com/big")
    assert "...[web content truncated: exceeded 50000 characters]" in md_output
    assert len(md_output) < 55000


def test_fetch_url_invalid_scheme():
    with pytest.raises(SystemExit) as exc_info:
        fetch_url("file:///etc/passwd")

    assert "Invalid URL scheme" in str(exc_info.value)
