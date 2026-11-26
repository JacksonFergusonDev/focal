use crate::file::{MAX_BYTES, Outcome, fenced};
use regex::Regex;
use serde_json::Value;
use std::fmt::Write;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::LazyLock;

// CSI (ECMA-48), OSC (BEL or ST terminated), nF escapes such as charset selection, and
// remaining two-byte escapes. Link text between OSC 8 markers is left in place.
static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[ -/]+[0-~]|[0-~])")
        .unwrap()
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static HIDDEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<!--.*?-->|<style\b.*?</style\s*>|<script\b.*?</script\s*>").unwrap()
});
static BLOCK_BREAK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<br\s*/?>|</(?:p|div|tr|li|h[1-6]|table|ul|ol|pre|blockquote)\s*>").unwrap()
});
static CELL_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</t[dh]\s*>").unwrap());
static PRE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<pre\b.*?</pre\s*>").unwrap());
static LEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^ +").unwrap());
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[a-zA-Z]+);").unwrap());
static REPR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^<[\w.]+ object(?: at 0x[0-9a-fA-F]+)?>$").unwrap());
static DATA_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bdata:([A-Za-z0-9.+-]+/[A-Za-z0-9.+-]+)((?:;[A-Za-z0-9=.+-]+)*);base64,[A-Za-z0-9+/_=-]{64,}")
        .unwrap()
});

/// Replaces large inline base64 `data:` URIs with a short marker, keeping the
/// surrounding markdown/HTML (alt text, tag attributes) intact.
fn omit_data_uris(text: &str) -> String {
    DATA_URI
        .replace_all(text, |caps: &regex::Captures| {
            let payload = caps[0].len() - caps[1].len() - caps[2].len() - "data:;base64,".len();
            format!(
                "data:{}{};base64,[omitted {payload} chars]",
                &caps[1], &caps[2]
            )
        })
        .into_owned()
}

fn joined(value: &Value) -> String {
    if let Some(lines) = value.as_array() {
        lines.iter().filter_map(Value::as_str).collect()
    } else {
        value.as_str().unwrap_or_default().to_owned()
    }
}

/// Emulate a terminal for carriage returns: `\r\n` is a newline, and within a line only
/// the text after the last bare `\r` survives (whole-line overwrite, no partial overwrite).
fn carriage_returns(text: &str) -> String {
    text.replace("\r\n", "\n")
        .split('\n')
        .map(|line| {
            line.rsplit('\r')
                .find(|part| !part.is_empty())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn stream_text(text: &str) -> String {
    carriage_returns(&ANSI.replace_all(text, ""))
}

/// Merge consecutive stream outputs with the same name into one, as nbconvert does.
fn merged_outputs(outputs: &[Value]) -> Vec<Value> {
    let mut merged: Vec<Value> = Vec::new();
    for output in outputs {
        if output["output_type"] == "stream"
            && let Some(last) = merged.last_mut()
            && last["output_type"] == "stream"
            && last["name"] == output["name"]
        {
            let text = format!("{}{}", joined(&last["text"]), joined(&output["text"]));
            last["text"] = Value::String(text);
        } else {
            merged.push(output.clone());
        }
    }
    merged
}

fn nonblank(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Collapses control characters (including newlines) to spaces so a value stays on one line.
fn one_line(value: &str) -> String {
    let spaced: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reduces a value to a safe fence info string: its first word, restricted to `[A-Za-z0-9_+#.-]`.
fn fence_token(value: &str) -> String {
    let token: String = value
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '#' | '.' | '-'))
        .collect();
    if token.is_empty() {
        "text".into()
    } else {
        token
    }
}

/// Maps a raw cell's `raw_mimetype` (or legacy `format`) to a fence language.
fn raw_language(metadata: &Value) -> String {
    let Some(value) = nonblank(&metadata["raw_mimetype"]).or_else(|| nonblank(&metadata["format"]))
    else {
        return "text".into();
    };
    let value = value.to_lowercase();
    let mime = value.split(';').next().unwrap_or_default().trim();
    match mime {
        "text/latex" | "application/x-latex" => "latex".into(),
        "text/restructuredtext" | "text/x-rst" => "rst".into(),
        "text/html" => "html".into(),
        "text/markdown" => "markdown".into(),
        "text/x-python" => "python".into(),
        "text/asciidoc" => "asciidoc".into(),
        _ => match mime.split_once('/') {
            Some((_, subtype)) => fence_token(subtype.trim_start_matches("x-")),
            None => fence_token(mime),
        },
    }
}

fn truncated(text: &str) -> String {
    match text.char_indices().nth(4000) {
        Some((end, _)) => format!("{}\n...[output truncated]", &text[..end]),
        None => text.to_owned(),
    }
}

fn output_block(body: &str, language: &str, prefix: Option<&str>) -> Option<String> {
    let text = truncated(body);
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    Some(match prefix {
        Some(label) => fenced(&format!("[{label}]\n{text}"), language),
        None => fenced(text, language),
    })
}

fn json_output(value: &Value) -> String {
    match value {
        Value::Object(_) | Value::Array(_) => {
            // Match the legacy pretty-printer's ASCII escapes before counting output characters.
            let mut text = String::new();
            for c in serde_json::to_string_pretty(value).unwrap().chars() {
                if c.is_ascii() && c != '\u{7f}' {
                    text.push(c);
                } else {
                    for unit in c.encode_utf16(&mut [0; 2]) {
                        write!(text, "\\u{unit:04x}").unwrap();
                    }
                }
            }
            text
        }
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
    }
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            let c = char::from_u32(code)?;
            Some(if c == '\u{a0}' { ' ' } else { c })
        }
    }
}

fn html_to_text(html: &str) -> String {
    let html = HIDDEN.replace_all(html, "");
    // Whitespace inside <pre> is significant: encode it so the collapse below keeps it.
    let html = PRE.replace_all(&html, |caps: &regex::Captures| {
        caps[0]
            .replace('\n', "<br>")
            .replace(' ', "&#32;")
            .replace('\t', "&#9;")
    });
    // Elsewhere HTML whitespace is insignificant; only tags create line breaks.
    let html = html.split_whitespace().collect::<Vec<_>>().join(" ");
    let html = BLOCK_BREAK.replace_all(&html, "\n");
    let html = CELL_END.replace_all(&html, "\t");
    let text = TAG.replace_all(&html, "");
    let text = LEADING.replace_all(&text, "");
    let text = ENTITY.replace_all(&text, |caps: &regex::Captures| {
        decode_entity(&caps[1]).map_or_else(|| caps[0].to_owned(), String::from)
    });
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_output(output: &Value) -> Option<String> {
    match output["output_type"].as_str()? {
        "stream" => output_block(
            &stream_text(&joined(&output["text"])),
            "text",
            Some(output["name"].as_str().unwrap_or("stdout")),
        ),
        "error" => {
            let description = format!(
                "{}: {}",
                output["ename"].as_str().unwrap_or(""),
                output["evalue"].as_str().unwrap_or("")
            );
            let mut body = description.trim_matches([':', ' ']).to_owned();
            if let Some(lines) = output["traceback"].as_array() {
                let trace = lines
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
                if !trace.trim().is_empty() {
                    body.push('\n');
                    body.push_str(&trace);
                }
            }
            output_block(&ANSI.replace_all(&body, ""), "text", Some("error"))
        }
        "display_data" | "execute_result" => {
            let data = output["data"].as_object()?;
            if data.keys().any(|key| key.starts_with("image/")) {
                return Some("[image output omitted]".into());
            }
            let mut order = [
                ("text/markdown", "markdown"),
                ("text/plain", "text"),
                ("text/html", "text"),
                ("text/latex", "latex"),
                ("application/json", "json"),
            ];
            // A bare object repr is a poor rendering; try richer types before it.
            if data
                .get("text/plain")
                .is_some_and(|plain| REPR.is_match(joined(plain).trim()))
            {
                order[1..].rotate_left(1);
            }
            for (mime, lang) in order {
                if let Some(value) = data.get(mime) {
                    let body = match mime {
                        "text/html" => html_to_text(&joined(value)),
                        "application/json" => json_output(value),
                        "text/markdown" => omit_data_uris(&joined(value)),
                        _ => joined(value),
                    };
                    if let Some(block) = output_block(&body, lang, None) {
                        return Some(block);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Rewrites an nbformat 3 notebook into the v4 shape so `render` handles both.
/// Mirrors nbformat's own v3 -> v4 upgrade for the fields focal renders.
fn upgrade_v3(nb: &mut Value) {
    let mut cells = Vec::new();
    if let Some(worksheets) = nb["worksheets"].as_array() {
        for sheet in worksheets {
            if let Some(list) = sheet["cells"].as_array() {
                cells.extend(list.iter().cloned());
            }
        }
    }
    let mut language = None;
    for cell in &mut cells {
        let Some(map) = cell.as_object_mut() else {
            continue;
        };
        match map.get("cell_type").and_then(Value::as_str) {
            Some("heading") => {
                let level = map.get("level").and_then(Value::as_u64).unwrap_or(1);
                let source = joined(map.get("source").unwrap_or(&Value::Null));
                let source = format!("{} {source}", "#".repeat(level.clamp(1, 6) as usize));
                map.insert("cell_type".into(), "markdown".into());
                map.insert("source".into(), source.into());
            }
            Some("code") => {
                if let Some(input) = map.remove("input") {
                    map.insert("source".into(), input);
                }
                let count = map.remove("prompt_number").unwrap_or(Value::Null);
                map.insert("execution_count".into(), count);
                if language.is_none() {
                    language = nonblank(&map["language"]).map(str::to_owned);
                }
                if let Some(outputs) = map.get_mut("outputs").and_then(Value::as_array_mut) {
                    outputs.iter_mut().for_each(upgrade_v3_output);
                }
            }
            _ => {}
        }
    }
    nb["cells"] = Value::Array(cells);
    if let Some(language) = language
        && nb["metadata"].is_object()
        && nb["metadata"]["language_info"].is_null()
    {
        nb["metadata"]["language_info"] = serde_json::json!({ "name": language });
    }
}

fn upgrade_v3_output(output: &mut Value) {
    let Some(map) = output.as_object_mut() else {
        return;
    };
    match map.get("output_type").and_then(Value::as_str) {
        Some("stream") => {
            if let Some(name) = map.remove("stream") {
                map.insert("name".into(), name);
            }
        }
        Some("pyerr") => {
            map.insert("output_type".into(), "error".into());
        }
        Some(kind @ ("pyout" | "display_data")) => {
            if kind == "pyout" {
                map.insert("output_type".into(), "execute_result".into());
            }
            let mut data = serde_json::Map::new();
            for (short, mime) in [
                ("text", "text/plain"),
                ("html", "text/html"),
                ("latex", "text/latex"),
                ("json", "application/json"),
                ("javascript", "application/javascript"),
                ("png", "image/png"),
                ("jpeg", "image/jpeg"),
                ("svg", "image/svg+xml"),
                ("pdf", "application/pdf"),
            ] {
                if let Some(value) = map.remove(short) {
                    let value = match (short, &value) {
                        ("json", Value::String(s)) => serde_json::from_str(s).unwrap_or(value),
                        _ => value,
                    };
                    data.insert(mime.into(), value);
                }
            }
            map.insert("data".into(), Value::Object(data));
        }
        _ => {}
    }
}

/// Render a notebook as markdown, capped at `MAX_BYTES`. A notebook that fails to parse is
/// reported as [`Outcome::Failed`]; one that had to be cut short as [`Outcome::Truncated`].
pub fn render(path: &Path, file: File) -> (String, Outcome) {
    let mut text = format!("# Notebook: {}\n\n", crate::file::shown(path));
    let nb = match serde_json::from_reader::<_, Value>(BufReader::new(file)) {
        Ok(mut nb) if nb.is_object() => {
            if nb["nbformat"].as_u64() == Some(3) {
                upgrade_v3(&mut nb);
            }
            nb
        }
        Ok(_) => {
            let note = "[Error parsing notebook: Invalid notebook format]\n\n";
            return (format!("{text}{note}"), Outcome::Failed);
        }
        Err(err) => {
            let note = format!("[Error parsing notebook: {err}]\n\n");
            return (format!("{text}{note}"), Outcome::Failed);
        }
    };
    let meta = &nb["metadata"];
    let kernel = &meta["kernelspec"];
    let lang = nonblank(&kernel["language"])
        .or_else(|| nonblank(&meta["language_info"]["name"]))
        .unwrap_or("python")
        .to_lowercase();
    let fence_lang = fence_token(&lang);
    let lang = one_line(&lang);
    let display = one_line(
        nonblank(&kernel["display_name"])
            .or_else(|| nonblank(&kernel["name"]))
            .unwrap_or("Unknown"),
    );
    let format = match (nb["nbformat"].as_u64(), nb["nbformat_minor"].as_u64()) {
        (Some(major), Some(minor)) => format!("nbformat {major}.{minor}"),
        (Some(major), None) => format!("nbformat {major}"),
        _ => "Unknown".into(),
    };
    let cells = nb["cells"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let count = |kind: &str| cells.iter().filter(|c| c["cell_type"] == kind).count();
    let raw = count("raw");
    let mut breakdown = format!("{} code, {} markdown", count("code"), count("markdown"));
    if raw > 0 {
        write!(breakdown, ", {raw} raw").unwrap();
    }
    let total = cells.iter().filter(|c| c.is_object()).count();
    let other = total - count("code") - count("markdown") - raw;
    if other > 0 {
        write!(breakdown, ", {other} other").unwrap();
    }
    write!(text, "## Notebook Metadata\n\n- Kernel: {display}\n- Language: {lang}\n- Format: {format}\n- Cells: {total} total ({breakdown})").unwrap();

    let mut boundaries = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let mut source = joined(&cell["source"]);
        if cell["cell_type"] == "markdown" {
            source = omit_data_uris(&source);
        }
        let source = source.trim_end();
        if source.is_empty() {
            continue;
        }
        let number = index + 1;
        let (heading, language) = match cell["cell_type"].as_str() {
            Some("markdown") => (format!("Markdown cell {number}"), "markdown".into()),
            Some("raw") => (
                format!("Raw cell {number}"),
                raw_language(&cell["metadata"]),
            ),
            Some("code") => {
                let execution = match &cell["execution_count"] {
                    Value::Null => "not executed".into(),
                    n => format!("execution: {n}"),
                };
                (
                    format!("Code cell {number} [{execution}]"),
                    fence_lang.clone(),
                )
            }
            _ => continue,
        };
        boundaries.push(text.len());
        write!(text, "\n\n## {heading}\n\n{}", fenced(source, &language)).unwrap();
        if cell["cell_type"] == "code"
            && let Some(outputs) = cell["outputs"].as_array()
        {
            for block in merged_outputs(outputs).iter().filter_map(render_output) {
                write!(text, "\n\n### Output\n\n{block}").unwrap();
            }
        }
    }
    if text.len() > MAX_BYTES {
        let total = text.len();
        let cut = boundaries
            .iter()
            .rev()
            .copied()
            .find(|&at| at <= MAX_BYTES)
            .unwrap_or_else(|| {
                (0..=MAX_BYTES)
                    .rev()
                    .find(|&at| text.is_char_boundary(at))
                    .unwrap_or(0)
            });
        text.truncate(cut);
        write!(
            text,
            "\n\n...[notebook truncated: showing first {cut} of {total} bytes]\n\n"
        )
        .unwrap();
        return (text, Outcome::Truncated);
    }
    text.push_str("\n\n");
    (text, Outcome::Text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn display(data: Value) -> Option<String> {
        render_output(&json!({"output_type": "display_data", "data": data}))
    }

    #[test]
    fn markdown_preferred_over_plain() {
        let out = display(json!({
            "text/markdown": ["# Title\n", "- a"],
            "text/plain": "<IPython.core.display.Markdown object>"
        }));
        assert_eq!(out.unwrap(), "```markdown\n# Title\n- a\n```");
    }

    #[test]
    fn markdown_only_is_kept() {
        let out = display(json!({"text/markdown": "**bold**"}));
        assert_eq!(out.unwrap(), "```markdown\n**bold**\n```");
    }

    #[test]
    fn plain_preferred_over_html_normally() {
        let out = display(json!({"text/plain": "   a\n0  1", "text/html": "<table></table>"}));
        assert_eq!(out.unwrap(), "```text\n   a\n0  1\n```");
    }

    #[test]
    fn repr_plain_yields_to_html_and_latex() {
        let out = display(json!({
            "text/plain": "<IPython.core.display.HTML object>",
            "text/html": "<b>hi</b>"
        }));
        assert_eq!(out.unwrap(), "```text\nhi\n```");
        let out = display(json!({
            "text/plain": "<IPython.core.display.Math object>",
            "text/latex": "$x^2$"
        }));
        assert_eq!(out.unwrap(), "```latex\n$x^2$\n```");
        // Without anything richer, the repr is still shown.
        let out = display(json!({"text/plain": "<Foo object at 0x10>"}));
        assert_eq!(out.unwrap(), "```text\n<Foo object at 0x10>\n```");
    }

    #[test]
    fn html_drops_style_script_comments() {
        let html = "<style>.a{color:red}</style><!-- note --><script>var x=1;</script><p>kept</p>";
        assert_eq!(html_to_text(html), "kept");
    }

    #[test]
    fn html_decodes_entities() {
        assert_eq!(
            html_to_text("a &lt; b &amp; c &quot;d&quot; &#39;e&#39; &#x41;&#66;&nbsp;x &bogus;"),
            "a < b & c \"d\" 'e' AB x &bogus;"
        );
        // No double decoding.
        assert_eq!(html_to_text("&amp;lt;"), "&lt;");
    }

    #[test]
    fn html_block_breaks_and_cells() {
        let html = "<h1>T</h1><div>one<br>two</div><ul><li>a</li><li>b</li></ul>\
            <table><tr><th>x</th><th>y</th></tr><tr><td>1</td><td>2</td></tr></table>";
        assert_eq!(html_to_text(html), "T\none\ntwo\na\nb\nx\ty\n1\t2");
    }

    #[test]
    fn html_pre_keeps_line_breaks_and_indentation() {
        let html = "<p>Code:</p>\n<pre class=\"x\">def f():\n    return 1 &lt; 2\n</pre>";
        assert_eq!(html_to_text(html), "Code:\ndef f():\n    return 1 < 2");
    }

    #[test]
    fn html_empty_leading_cell_keeps_alignment() {
        let html = "<table><tr><td></td><td>b</td></tr></table>";
        assert_eq!(html_to_text(html), "\tb");
    }

    fn rendered(nb: &Value) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.ipynb");
        fs::write(&path, nb.to_string()).unwrap();
        render(Path::new("t.ipynb"), File::open(&path).unwrap()).0
    }

    #[test]
    fn nbformat3_matches_v4() {
        let v3 = json!({
            "nbformat": 3, "nbformat_minor": 0, "metadata": {"name": "x"},
            "worksheets": [{"cells": [
                {"cell_type": "heading", "level": 2, "source": ["Title"], "metadata": {}},
                {"cell_type": "markdown", "source": "text", "metadata": {}},
                {"cell_type": "code", "language": "python", "input": ["1+1"],
                 "prompt_number": 3, "metadata": {}, "outputs": [
                    {"output_type": "stream", "stream": "stdout", "text": ["hi\n"]},
                    {"output_type": "pyout", "prompt_number": 3, "text": ["2"], "metadata": {}},
                    {"output_type": "pyerr", "ename": "E", "evalue": "bad", "traceback": ["tb"]},
                    {"output_type": "display_data", "png": "AAAA", "text": ["<Fig>"], "metadata": {}}
                ]}
            ]}]
        });
        let v4 = json!({
            "nbformat": 4, "nbformat_minor": 0,
            "metadata": {"language_info": {"name": "python"}},
            "cells": [
                {"cell_type": "markdown", "source": "## Title", "metadata": {}},
                {"cell_type": "markdown", "source": "text", "metadata": {}},
                {"cell_type": "code", "source": ["1+1"], "execution_count": 3, "metadata": {}, "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["hi\n"]},
                    {"output_type": "execute_result", "execution_count": 3,
                     "data": {"text/plain": ["2"]}, "metadata": {}},
                    {"output_type": "error", "ename": "E", "evalue": "bad", "traceback": ["tb"]},
                    {"output_type": "display_data",
                     "data": {"image/png": "AAAA", "text/plain": ["<Fig>"]}, "metadata": {}}
                ]}
            ]
        });
        let a = rendered(&v3);
        let b = rendered(&v4);
        assert!(a.contains("Cells: 3 total (1 code, 2 markdown)"));
        assert!(a.contains("## Title"));
        assert!(a.contains("execution: 3"));
        assert!(a.contains("[stdout]\nhi"));
        assert!(a.contains("[error]\nE: bad"));
        assert!(a.contains("[image output omitted]"));
        assert!(a.contains("- Language: python"));
        assert_eq!(
            a.replace("nbformat 3.0", "F"),
            b.replace("nbformat 4.0", "F")
        );
    }

    #[test]
    fn unknown_cell_types_are_counted() {
        let nb = json!({"nbformat": 4, "nbformat_minor": 5, "metadata": {}, "cells": [
            {"cell_type": "code", "source": "x", "execution_count": null, "outputs": []},
            {"cell_type": "mystery", "source": "y"}
        ]});
        assert!(rendered(&nb).contains("Cells: 2 total (1 code, 0 markdown, 1 other)"));
    }

    #[test]
    fn omits_markdown_image_payload() {
        let payload = "A".repeat(200);
        let out = omit_data_uris(&format!("![plot](data:image/png;base64,{payload}) after"));
        assert_eq!(
            out,
            "![plot](data:image/png;base64,[omitted 200 chars]) after"
        );
    }

    #[test]
    fn omits_html_img_payload() {
        let payload = "QUJD".repeat(100);
        let out = omit_data_uris(&format!(
            r#"<img alt="x" src="data:image/jpeg;base64,{payload}" width="5">"#
        ));
        assert_eq!(
            out,
            r#"<img alt="x" src="data:image/jpeg;base64,[omitted 400 chars]" width="5">"#
        );
    }

    #[test]
    fn keeps_short_uris_and_plain_text() {
        let text = "![d](data:image/png;base64,AAAA) and https://x.y/z";
        assert_eq!(omit_data_uris(text), text);
    }

    #[test]
    fn render_only_touches_markdown_cells() {
        let payload = "A".repeat(100);
        let uri = format!("data:image/png;base64,{payload}");
        let nb = serde_json::json!({"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[
            {"cell_type":"markdown","source":[format!("![a]({uri})")]},
            {"cell_type":"code","execution_count":null,"outputs":[],"source":[format!("u = '{uri}'")]},
        ]});
        let dir = std::env::temp_dir().join(format!("focal-nb-{}.ipynb", std::process::id()));
        std::fs::write(&dir, nb.to_string()).unwrap();
        let out = super::render(&dir, std::fs::File::open(&dir).unwrap()).0;
        std::fs::remove_file(&dir).unwrap();
        assert!(out.contains("![a](data:image/png;base64,[omitted 100 chars])"));
        assert!(out.contains(&format!("u = '{uri}'")));
    }

    #[test]
    fn ignores_data_inside_words() {
        let text = format!("metadata:image/png;base64,{}", "A".repeat(100));
        assert_eq!(omit_data_uris(&text), text);
    }

    #[test]
    fn omits_payload_in_markdown_outputs() {
        let uri = format!("data:image/png;base64,{}", "A".repeat(100));
        let out = render_output(&json!({
            "output_type": "display_data",
            "data": {"text/markdown": format!("![p]({uri})")},
        }))
        .unwrap();
        assert!(
            out.contains("![p](data:image/png;base64,[omitted 100 chars])"),
            "{out}"
        );
    }

    fn stream(name: &str, text: &str) -> Value {
        json!({"output_type": "stream", "name": name, "text": text})
    }

    #[test]
    fn strips_extended_escapes() {
        for (input, want) in [
            ("\x1b[?25lhi\x1b[?25h", "hi"),
            ("\x1b[31;1mred\x1b[0m", "red"),
            ("\x1b]8;;http://x\x07link\x1b]8;;\x07", "link"),
            ("\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\", "link"),
            ("\x1b(Bplain\x1b=", "plain"),
            ("a\x1b[2Kb", "ab"),
        ] {
            assert_eq!(ANSI.replace_all(input, ""), want, "{input:?}");
        }
    }

    #[test]
    fn carriage_returns_keep_final_state() {
        assert_eq!(
            carriage_returns("  0%|  |\r 33%|# |\r100%|##|\nnext\r\nend"),
            "100%|##|\nnext\nend"
        );
        assert_eq!(carriage_returns("abc\r"), "abc");
    }

    #[test]
    fn merges_consecutive_same_name_streams() {
        let outputs = [
            stream("stdout", "a\n"),
            stream("stdout", "b\n"),
            stream("stderr", "e\n"),
            stream("stdout", "c\n"),
        ];
        let merged = merged_outputs(&outputs);
        assert_eq!(merged.len(), 3);
        assert_eq!(
            render_output(&merged[0]).unwrap(),
            fenced("[stdout]\na\nb", "text")
        );
    }

    #[test]
    fn cr_applies_after_merge_and_truncation_is_on_merged_text() {
        let outputs = [stream("stdout", "10%\r"), stream("stdout", "100%\n")];
        let merged = merged_outputs(&outputs);
        assert_eq!(
            render_output(&merged[0]).unwrap(),
            fenced("[stdout]\n100%", "text")
        );

        let outputs = [
            stream("stdout", &"x".repeat(3000)),
            stream("stdout", &"y".repeat(3000)),
        ];
        let merged = merged_outputs(&outputs);
        assert_eq!(merged.len(), 1);
        let block = render_output(&merged[0]).unwrap();
        assert!(block.contains("[output truncated]"));
        assert!(block.contains('x') && block.contains('y'));
    }

    #[test]
    fn sanitizes_inputs() {
        assert_eq!(fence_token("py thon\n# injected"), "py");
        assert_eq!(fence_token("c++"), "c++");
        assert_eq!(fence_token("```\n"), "text");
        assert_eq!(one_line("a\n# b\r\tc"), "a # b c");
        assert_eq!(one_line("Python 3 (ipykernel)"), "Python 3 (ipykernel)");
    }

    #[test]
    fn raw_mimetypes() {
        let lang = |m: Value| raw_language(&m);
        assert_eq!(lang(json!({"raw_mimetype": "text/latex"})), "latex");
        assert_eq!(
            lang(json!({"raw_mimetype": "text/restructuredtext"})),
            "rst"
        );
        assert_eq!(lang(json!({"raw_mimetype": "text/x-rst"})), "rst");
        assert_eq!(lang(json!({"raw_mimetype": "text/html"})), "html");
        assert_eq!(lang(json!({"raw_mimetype": "text/markdown"})), "markdown");
        assert_eq!(lang(json!({"raw_mimetype": "text/x-python"})), "python");
        assert_eq!(lang(json!({"raw_mimetype": "text/asciidoc"})), "asciidoc");
        assert_eq!(lang(json!({"raw_mimetype": "text/x-foo"})), "foo");
        assert_eq!(lang(json!({"raw_mimetype": "text/"})), "text");
        assert_eq!(lang(json!({"format": "latex"})), "latex");
        assert_eq!(
            lang(json!({"raw_mimetype": "text/html", "format": "latex"})),
            "html"
        );
        assert_eq!(lang(json!({})), "text");
    }

    #[test]
    fn injected_metadata_stays_inline() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let nb = json!({
            "nbformat": 4,
            "metadata": {"kernelspec": {
                "language": "py thon\n# injected heading",
                "display_name": "K\n## Evil"
            }},
            "cells": [{"cell_type": "code", "source": "x"}]
        });
        std::fs::write(tmp.path(), nb.to_string()).unwrap();
        let out = render(tmp.path(), File::open(tmp.path()).unwrap()).0;
        assert!(out.contains("- Kernel: K ## Evil\n"));
        assert!(out.contains("- Language: py thon # injected heading\n"));
        assert!(!out.contains("\n# injected"));
        assert!(out.contains("```py\nx\n```"));
    }
}
