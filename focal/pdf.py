"""Extracts text content from PDF documents for LLM context generation."""

import sys

import pdfplumber

from focal.errors import die

MAX_PAGES = 50
MAX_OUTPUT_CHARS = 50000


def pdf_to_llm_text(path: str) -> str:
    """Converts a PDF document into an LLM-optimized text string.

    Uses pdfplumber to extract text while maintaining spatial layout and tables.
    Includes safeguards to prevent massive documents from exceeding context windows.

    Args:
        path: The file system path to the target `.pdf` file.

    Returns:
        The extracted text representation of the PDF.
    """
    try:
        with pdfplumber.open(path) as pdf:
            pages = []
            for i, page in enumerate(pdf.pages):
                if i >= MAX_PAGES:
                    pages.append(f"\n...[PDF truncated: max {MAX_PAGES} pages reached]")
                    break
                text = page.extract_text()
                if text:
                    pages.append(text)

            content = "\n\n".join(pages)
            if len(content) > MAX_OUTPUT_CHARS:
                content = (
                    content[:MAX_OUTPUT_CHARS]
                    + f"\n...[PDF truncated: exceeded {MAX_OUTPUT_CHARS} characters]"
                )

            return content
    except Exception as e:
        return f"[Error parsing PDF: {e}]"


def main() -> None:
    """Executes the CLI script to parse a PDF for LLM context.

    Reads a PDF path from system arguments and writes the extracted
    text directly to standard output.

    Raises:
        SystemExit: If the exact required number of arguments is not provided.
    """
    if len(sys.argv) != 2:
        die("missing PDF path argument", hint="usage: python -m focal.pdf file.pdf")

    sys.stdout.write(pdf_to_llm_text(sys.argv[1]) + "\n")


if __name__ == "__main__":
    main()
