"""Parses Jupyter notebook (.ipynb) files into clean, LLM-optimized markdown representations."""

import json

MAX_OUTPUT_CHARS = 4000


def join_text(x: list[str] | str | None) -> str:
    """Normalizes Jupyter notebook text fields into a single continuous string.

    Jupyter notebook JSON formats often represent multi-line text blocks as
    lists of strings. This function safely concatenates them.

    Args:
        x: The text field payload from the notebook cell.

    Returns:
        The concatenated string, or an empty string if the input is None.
    """
    if isinstance(x, list):
        return "".join(x)
    return x or ""


def truncate(text: str) -> str:
    """Truncates text to prevent context window overflow.

    Args:
        text: The raw output string to be evaluated.

    Returns:
        The original string if its length is within `MAX_OUTPUT_CHARS`,
        otherwise a truncated slice appended with an omission notice.
    """
    if len(text) > MAX_OUTPUT_CHARS:
        return text[:MAX_OUTPUT_CHARS] + "\n...[output truncated]"
    return text


def render_output(out: dict) -> str | None:
    """Parses and formats a Jupyter cell output dictionary into markdown.

    Extracts stdout streams, error tracebacks, and plain text execution results
    while explicitly omitting binary/image data types.

    Args:
        out: A single output payload from a Jupyter notebook code cell.

    Returns:
        A formatted markdown string representing the cell output, or None if the
        output type is unsupported or completely empty.
    """
    ot = out.get("output_type")

    if ot == "stream":
        name = out.get("name", "stdout")
        text = truncate(join_text(out.get("text", "")))
        if text.strip():
            return f"```text\n[{name}]\n{text.rstrip()}\n```"

    if ot == "error":
        ename = out.get("ename", "")
        evalue = out.get("evalue", "")
        tb = "\n".join(out.get("traceback", []))

        body = f"{ename}: {evalue}".strip(": ")

        if tb.strip():
            body = f"{body}\n{tb}"

        body = truncate(body)

        if body.strip():
            return f"```text\n[error]\n{body.rstrip()}\n```"

    if ot in {"display_data", "execute_result"}:
        data = out.get("data", {})

        if any(k.startswith("image/") for k in data):
            return "[image output omitted]"

        text = truncate(join_text(data.get("text/plain", "")))

        if text.strip():
            return f"```text\n{text.rstrip()}\n```"

    return None


def notebook_to_llm_text(path: str) -> str:
    """Converts a complete Jupyter notebook into an LLM-optimized markdown document.

    Iterates sequentially through the notebook's AST, extracting markdown cells,
    code cells, and their corresponding execution outputs, wrapping them in
    standard markdown blocks.

    Args:
        path: The file system path to the target `.ipynb` file.

    Returns:
        The complete formatted markdown representation of the notebook.
    """
    with open(path, encoding="utf-8") as f:
        nb = json.load(f)

    parts = [f"# Notebook: {path}"]

    for i, cell in enumerate(nb.get("cells", []), start=1):
        ctype = cell.get("cell_type")

        if ctype == "markdown":
            src = join_text(cell.get("source")).rstrip()

            if src:
                block = [
                    f"\n## Markdown cell {i}",
                    "",
                    "```markdown",
                    src,
                    "```",
                ]
                parts.append("\n".join(block))

        elif ctype == "code":
            src = join_text(cell.get("source")).rstrip()

            block = [
                f"\n## Code cell {i}",
                "",
                "```python",
                src,
                "```",
            ]

            for out in cell.get("outputs", []):
                rendered = render_output(out)

                if rendered:
                    block.append("\n### Output\n")
                    block.append(rendered)

            parts.append("\n".join(block))

    return "\n".join(parts).strip() + "\n"
