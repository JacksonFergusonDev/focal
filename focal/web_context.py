"""Fetches and converts web pages or raw HTML input into clean markdown."""

import re
import urllib.error
import urllib.request

from bs4 import BeautifulSoup
from markdownify import markdownify

from focal.errors import die


def parse_html_to_md(html: str, source_label: str) -> str:
    """Strips noisy DOM elements from HTML and returns clean Markdown.

    Args:
        html: Raw HTML content string.
        source_label: A label or URL identifying the source of the HTML.

    Returns:
        A markdown representation of the cleaned HTML page content.
    """
    soup = BeautifulSoup(html, "html.parser")

    # Only strip tags that are explicitly designed for non-content or execution
    noise_tags = [
        "script",
        "style",
        "nav",
        "footer",
        "aside",
        "header",
        "meta",
        "noscript",
        "svg",
        "form",
        "iframe",
        "button",
    ]
    for element in soup(noise_tags):
        element.decompose()

    md = markdownify(str(soup), heading_style="ATX", default_title=True)

    # Clean up the whitespace graveyard left by decomposed tags
    md = re.sub(r"\n{3,}", "\n\n", md).strip()

    return f"# Source: {source_label}\n\n{md}\n"


def fetch_url(url: str) -> str:
    """Fetches raw HTML from a public URL.

    Args:
        url: The web URL to retrieve.

    Returns:
        The decoded HTML string fetched from the URL.

    Raises:
        SystemExit: If an HTTP error or network failure occurs.
    """
    req = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as response:
            raw_bytes: bytes = response.read()
            charset = response.headers.get_content_charset() or "utf-8"
            return raw_bytes.decode(charset, errors="replace")
    except urllib.error.URLError as e:
        die(
            f"Error fetching {url}: {e}",
            hint="check network connectivity or URL validity",
        )


def get_web_context(url: str | None = None, html_content: str | None = None) -> str:
    """Generates markdown context from a URL or raw HTML string.

    Args:
        url: Optional URL to fetch.
        html_content: Optional raw HTML string.

    Returns:
        Clean markdown representation of the web page.

    Raises:
        SystemExit: If neither url nor valid html_content is provided.
    """
    if html_content is not None:
        if not html_content.strip():
            die(
                "received empty piped input",
                hint="pipe HTML via stdin or provide a URL argument",
            )
        return parse_html_to_md(html_content, "Piped DOM/Clipboard")

    if url:
        raw_html = fetch_url(url)
        return parse_html_to_md(raw_html, url)

    die(
        "missing URL or HTML input",
        hint="usage: pbpaste | focal web OR focal web <url>",
    )
    return ""
