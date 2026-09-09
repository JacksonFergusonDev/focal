import json
from unittest.mock import mock_open, patch

from focal import notebook


def test_join_text_normalization():
    assert notebook.join_text(["line 1\n", "line 2"]) == "line 1\nline 2"
    assert notebook.join_text("string block") == "string block"
    assert notebook.join_text(None) == ""


def test_truncate_respects_limits(monkeypatch):
    # Monkeypatch the module constant to avoid generating 4000 char strings
    monkeypatch.setattr(notebook, "MAX_OUTPUT_CHARS", 10)

    assert notebook.truncate("short") == "short"
    assert notebook.truncate("1234567890_extra") == "1234567890\n...[output truncated]"
    # Test explicit max_chars parameter
    assert (
        notebook.truncate("1234567890", max_chars=5) == "12345\n...[output truncated]"
    )


def test_render_output_stream():
    payload = {"output_type": "stream", "name": "stdout", "text": ["hello\n", "world"]}
    result = notebook.render_output(payload)
    assert result == "```text\n[stdout]\nhello\nworld\n```"


def test_render_output_ignores_images():
    payload = {
        "output_type": "display_data",
        "data": {"image/png": "iVBORw0KGgo...", "text/plain": "<matplotlib.axes>"},
    }
    result = notebook.render_output(payload)
    assert result == "[image output omitted]"


def test_render_output_stream_empty():
    payload = {"output_type": "stream", "name": "stdout", "text": ["   \n"]}
    result = notebook.render_output(payload)
    assert result is None


def test_render_output_error():
    payload = {
        "output_type": "error",
        "ename": "ValueError",
        "evalue": "bad value",
        "traceback": ["Traceback (most recent call last):", "  File ..."],
    }
    result = notebook.render_output(payload)
    assert (
        result
        == "```text\n[error]\nValueError: bad value\nTraceback (most recent call last):\n  File ...\n```"
    )


def test_strip_ansi():
    text_with_ansi = "\x1b[31;1mError:\x1b[0m \x1b[33mSomething failed\x1b[m"
    assert notebook.strip_ansi(text_with_ansi) == "Error: Something failed"


def test_render_output_error_strips_ansi():
    payload = {
        "output_type": "error",
        "ename": "\x1b[0;31mValueError\x1b[0m",
        "evalue": "\x1b[1minvalid literal\x1b[0m",
        "traceback": [
            "\x1b[0;31mTraceback (most recent call last):\x1b[0m",
            "  File \x1b[0;32m<stdin>\x1b[0m, line 1",
        ],
    }
    result = notebook.render_output(payload)
    assert result == (
        "```text\n[error]\nValueError: invalid literal\n"
        "Traceback (most recent call last):\n  File <stdin>, line 1\n```"
    )


def test_render_output_error_empty():
    payload = {"output_type": "error", "ename": "", "evalue": "", "traceback": []}
    result = notebook.render_output(payload)
    assert result is None


def test_render_output_execute_result():
    payload = {"output_type": "execute_result", "data": {"text/plain": "42"}}
    result = notebook.render_output(payload)
    assert result == "```text\n42\n```"


def test_render_output_execute_result_empty():
    payload = {"output_type": "execute_result", "data": {"text/plain": "  "}}
    result = notebook.render_output(payload)
    assert result is None


def test_render_output_html():
    payload = {
        "output_type": "display_data",
        "data": {
            "text/html": [
                "<div>\n",
                "  <table><tr><td>Item</td><td>Value</td></tr></table>\n",
                "</div>",
            ]
        },
    }
    result = notebook.render_output(payload)
    assert result is not None
    assert "Item" in result
    assert "Value" in result
    assert "<table" not in result
    assert result.startswith("```text\n")


def test_render_output_html_empty_after_strip():
    payload = {
        "output_type": "display_data",
        "data": {"text/html": "<div><span>   \n </span></div>"},
    }
    assert notebook.render_output(payload) is None


def test_render_output_latex():
    payload = {
        "output_type": "display_data",
        "data": {"text/latex": "\\frac{1}{2}"},
    }
    result = notebook.render_output(payload)
    assert result == "```latex\n\\frac{1}{2}\n```"


def test_render_output_json():
    payload = {
        "output_type": "display_data",
        "data": {"application/json": {"key": "val", "num": 123}},
    }
    result = notebook.render_output(payload)
    assert result is not None
    assert result.startswith("```json\n")
    assert '"key": "val"' in result
    assert '"num": 123' in result


def test_render_output_mime_priority():
    payload = {
        "output_type": "display_data",
        "data": {
            "application/json": {"k": "v"},
            "text/latex": "\\alpha",
            "text/html": "<b>bold</b>",
            "text/plain": "plain text preference",
        },
    }
    result = notebook.render_output(payload)
    assert result == "```text\nplain text preference\n```"


def test_render_output_unknown_type():
    payload = {"output_type": "unknown"}
    result = notebook.render_output(payload)
    assert result is None


def test_notebook_to_llm_text():
    mock_nb = {
        "cells": [
            {"cell_type": "markdown", "source": ["# Title\n", "Some text."]},
            {
                "cell_type": "code",
                "execution_count": 1,
                "source": ["print(1)"],
                "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["1\n"]}
                ],
            },
        ]
    }

    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("test.ipynb")

        assert "# Notebook: test.ipynb" in result
        assert "## Notebook Metadata" in result
        assert "## Markdown cell 1" in result
        assert "```markdown\n# Title\nSome text.\n```" in result
        assert "## Code cell 2 [execution: 1]" in result
        assert "```python\nprint(1)\n```" in result
        assert "### Output" in result
        assert "```text\n[stdout]\n1\n```" in result


def test_notebook_to_llm_text_custom_max_output():
    mock_nb = {
        "cells": [
            {
                "cell_type": "code",
                "execution_count": 1,
                "source": ["print('a' * 100)"],
                "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["a" * 100]}
                ],
            },
        ]
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("test_trunc.ipynb", max_output_chars=20)
        assert "...[output truncated]" in result
        assert "a" * 20 in result
        assert "a" * 21 not in result


def test_code_cell_execution_counts():
    mock_nb = {
        "cells": [
            {
                "cell_type": "code",
                "execution_count": 42,
                "source": ["a = 1"],
                "outputs": [],
            },
            {
                "cell_type": "code",
                "execution_count": None,
                "source": ["b = 2"],
                "outputs": [],
            },
        ]
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("counts.ipynb")
        assert "## Code cell 1 [execution: 42]" in result
        assert "## Code cell 2 [not executed]" in result


def test_empty_code_cell_skipped():
    mock_nb = {
        "cells": [
            {
                "cell_type": "code",
                "execution_count": None,
                "source": ["   \n\t  "],
                "outputs": [],
            },
            {
                "cell_type": "code",
                "execution_count": 1,
                "source": ["x = 10"],
                "outputs": [],
            },
        ]
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("empty.ipynb")
        assert "## Code cell 1" not in result
        assert "## Code cell 2 [execution: 1]" in result


def test_raw_cell_rendering():
    mock_nb = {
        "cells": [
            {
                "cell_type": "raw",
                "source": ["raw text content"],
            },
            {
                "cell_type": "raw",
                "source": ["   \n  "],
            },
        ]
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("raw.ipynb")
        assert "## Raw cell 1" in result
        assert "```text\nraw text content\n```" in result
        assert "## Raw cell 2" not in result


def test_raw_cell_with_format_metadata():
    mock_nb = {
        "cells": [
            {
                "cell_type": "raw",
                "metadata": {"format": "text/restructuredtext"},
                "source": [".. note::\n   hello"],
            }
        ]
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("raw_rst.ipynb")
        assert "## Raw cell 1" in result
        assert "```text/restructuredtext\n.. note::\n   hello\n```" in result


def test_notebook_to_llm_text_corrupted_json():
    with patch("builtins.open", mock_open(read_data="invalid json content")):
        result = notebook.notebook_to_llm_text("corrupted.ipynb")
        assert "# Notebook: corrupted.ipynb" in result
        assert "[Error parsing notebook:" in result


def test_notebook_to_llm_text_non_dict_json():
    with patch("builtins.open", mock_open(read_data="[1, 2, 3]")):
        result = notebook.notebook_to_llm_text("array.ipynb")
        assert "# Notebook: array.ipynb" in result
        assert "[Error parsing notebook: Invalid notebook format]" in result


def test_render_output_invalid_data():
    assert notebook.render_output(None) is None
    assert (
        notebook.render_output(
            {"output_type": "display_data", "data": "not a dictionary"}
        )
        is None
    )


def test_get_kernel_language_from_kernelspec():
    nb = {"metadata": {"kernelspec": {"language": "julia"}}}
    assert notebook.get_kernel_language(nb) == "julia"


def test_get_kernel_language_from_language_info():
    nb = {"metadata": {"language_info": {"name": "R"}}}
    assert notebook.get_kernel_language(nb) == "r"


def test_get_kernel_language_fallback_and_invalid():
    assert notebook.get_kernel_language({}) == "python"
    assert notebook.get_kernel_language({"metadata": "not_dict"}) == "python"
    assert notebook.get_kernel_language({"metadata": {"kernelspec": {}}}) == "python"


def test_code_cell_uses_kernel_language():
    mock_nb = {
        "metadata": {"kernelspec": {"language": "r"}},
        "cells": [
            {
                "cell_type": "code",
                "source": ["x <- c(1, 2, 3)"],
                "outputs": [],
            }
        ],
    }
    with patch("builtins.open", mock_open(read_data=json.dumps(mock_nb))):
        result = notebook.notebook_to_llm_text("test_r.ipynb")
        assert "```r\nx <- c(1, 2, 3)\n```" in result


def test_build_notebook_metadata_header():
    nb = {
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {
                "display_name": "Python 3 (ipykernel)",
                "name": "python3",
            }
        },
        "cells": [
            {"cell_type": "code"},
            {"cell_type": "code"},
            {"cell_type": "markdown"},
            {"cell_type": "raw"},
        ],
    }
    header = notebook.build_notebook_metadata_header(nb, "python")
    assert "## Notebook Metadata" in header
    assert "- Kernel: Python 3 (ipykernel)" in header
    assert "- Language: python" in header
    assert "- Format: nbformat 4.5" in header
    assert "- Cells: 4 total (2 code, 1 markdown, 1 raw)" in header


def test_build_notebook_metadata_header_defaults():
    header = notebook.build_notebook_metadata_header({}, "python")
    assert "- Kernel: Unknown" in header
    assert "- Format: Unknown" in header
    assert "- Cells: 0 total (0 code, 0 markdown)" in header
