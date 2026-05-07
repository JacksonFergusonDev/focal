import re
import sys
import urllib.error
import urllib.request

from bs4 import BeautifulSoup
from markdownify import markdownify


def parse_html_to_md(html: str, source_label: str) -> str:
    """Strips noisy DOM elements from HTML and returns clean Markdown."""
    soup = BeautifulSoup(html, "html.parser")

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
    ]
    for element in soup(noise_tags):
        element.decompose()

    md = markdownify(str(soup), heading_style="ATX", default_title=True)
    md = re.sub(r"\n{3,}", "\n\n", md).strip()

    return f"# Source: {source_label}\n\n{md}\n"


def fetch_url(url: str) -> str:
    """Fetches raw HTML from a public URL."""
    req = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
        },
    )
    try:
        with urllib.request.urlopen(req) as response:
            raw_bytes: bytes = response.read()
            return raw_bytes.decode("utf-8")
    except urllib.error.URLError as e:
        sys.exit(f"Error fetching {url}: {e}")


def main() -> None:
    """Executes the CLI script to parse a webpage or piped HTML for LLM context.

    Raises:
        SystemExit: If piped input is empty or if the incorrect number
            of CLI arguments is provided.
    """
    # Check if data is being piped in via stdin
    if not sys.stdin.isatty():
        raw_html = sys.stdin.read()
        if not raw_html.strip():
            sys.exit("Error: Received empty piped input.")
        sys.stdout.write(parse_html_to_md(raw_html, "Piped DOM/Clipboard"))
        return

    # Otherwise, expect a URL argument
    if len(sys.argv) != 2:
        sys.exit(
            "Usage: pbpaste | python -m focal.web_context OR python -m focal.web_context <url>"
        )

    url = sys.argv[1]
    raw_html = fetch_url(url)
    sys.stdout.write(parse_html_to_md(raw_html, url))


if __name__ == "__main__":
    main()
