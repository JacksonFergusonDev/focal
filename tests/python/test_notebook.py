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
