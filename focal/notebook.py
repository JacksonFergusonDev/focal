"""Parses Jupyter notebook (.ipynb) files into clean, LLM-optimized markdown representations."""

import json
from typing import Any

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


def render_output(out: Any) -> str | None:
    """Parses and formats a Jupyter cell output dictionary into markdown.

    Extracts stdout streams, error tracebacks, and plain text execution results
    while explicitly omitting binary/image data types.

    Args:
        out: A single output payload from a Jupyter notebook code cell.

    Returns:
        A formatted markdown string representing the cell output, or None if the
        output type is unsupported or completely empty.
    """
    if not isinstance(out, dict):
        return None

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
        data = out.get("data")
        if not isinstance(data, dict):
            return None

        if any(k.startswith("image/") for k in data):
            return "[image output omitted]"

        text = truncate(join_text(data.get("text/plain", "")))

        if text.strip():
            return f"```text\n{text.rstrip()}\n```"

    return None


def get_kernel_language(nb: dict[str, Any]) -> str:
    """Extracts the notebook programming language from kernelspec or language_info.

    Args:
        nb: The parsed notebook JSON dictionary.

    Returns:
        The detected language name in lowercase, or 'python' as default.
    """
    meta = nb.get("metadata")
    if not isinstance(meta, dict):
        return "python"

    ks = meta.get("kernelspec")
    if isinstance(ks, dict):
        lang = ks.get("language")
        if isinstance(lang, str) and lang.strip():
            return lang.strip().lower()

    li = meta.get("language_info")
    if isinstance(li, dict):
        name = li.get("name")
        if isinstance(name, str) and name.strip():
            return name.strip().lower()

    return "python"


def build_notebook_metadata_header(nb: dict[str, Any], lang: str) -> str:
    """Constructs a structured metadata summary of the notebook.

    Args:
        nb: The parsed notebook JSON dictionary.
        lang: The detected notebook programming language.

    Returns:
        Formatted markdown section containing key notebook metadata.
    """
    meta_raw = nb.get("metadata")
    meta = meta_raw if isinstance(meta_raw, dict) else {}
    ks_raw = meta.get("kernelspec")
    ks = ks_raw if isinstance(ks_raw, dict) else {}
    kernel_display = ks.get("display_name") or ks.get("name") or "Unknown"

    nbformat = nb.get("nbformat")
    nbformat_minor = nb.get("nbformat_minor")
    if nbformat is not None:
        format_str = (
            f"nbformat {nbformat}.{nbformat_minor}"
            if nbformat_minor is not None
            else f"nbformat {nbformat}"
        )
    else:
        format_str = "Unknown"

    cells_raw = nb.get("cells")
    cells = cells_raw if isinstance(cells_raw, list) else []
    counts: dict[str, int] = {}
    for cell in cells:
        if isinstance(cell, dict):
            ctype = cell.get("cell_type", "unknown")
            counts[ctype] = counts.get(ctype, 0) + 1

    total = sum(counts.values())
    code_count = counts.get("code", 0)
    md_count = counts.get("markdown", 0)
    raw_count = counts.get("raw", 0)

    breakdown = f"{code_count} code, {md_count} markdown"
    if raw_count > 0:
        breakdown += f", {raw_count} raw"

    lines = [
        "## Notebook Metadata",
        "",
        f"- Kernel: {kernel_display}",
        f"- Language: {lang}",
        f"- Format: {format_str}",
        f"- Cells: {total} total ({breakdown})",
    ]
    return "\n".join(lines)


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
    try:
        with open(path, encoding="utf-8") as f:
            nb = json.load(f)
    except Exception as e:
        return f"# Notebook: {path}\n\n[Error parsing notebook: {e}]\n"

    if not isinstance(nb, dict):
        return (
            f"# Notebook: {path}\n\n[Error parsing notebook: Invalid notebook format]\n"
        )

    lang = get_kernel_language(nb)
    parts = [f"# Notebook: {path}", "", build_notebook_metadata_header(nb, lang)]

    for i, cell in enumerate(nb.get("cells", []), start=1):
        if not isinstance(cell, dict):
            continue
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
            if not src.strip():
                continue

            exec_count = cell.get("execution_count")
            exec_str = (
                f"[execution: {exec_count}]"
                if exec_count is not None
                else "[not executed]"
            )

            block = [
                f"\n## Code cell {i} {exec_str}",
                "",
                f"```{lang}",
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
