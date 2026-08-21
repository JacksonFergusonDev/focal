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
        assert "## Markdown cell 1" in result
        assert "```markdown\n# Title\nSome text.\n```" in result
        assert "## Code cell 2" in result
        assert "```python\nprint(1)\n```" in result
        assert "### Output" in result
        assert "```text\n[stdout]\n1\n```" in result
