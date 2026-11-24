use crate::file::fenced;
use regex::Regex;
use serde_json::Value;
use std::fmt::Write;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::LazyLock;

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

fn joined(value: &Value) -> String {
    if let Some(lines) = value.as_array() {
        lines.iter().filter_map(Value::as_str).collect()
    } else {
        value.as_str().unwrap_or_default().to_owned()
    }
}

fn nonblank(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
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

fn render_output(output: &Value) -> Option<String> {
    match output["output_type"].as_str()? {
        "stream" => output_block(
            &ANSI.replace_all(&joined(&output["text"]), ""),
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
            for (mime, lang) in [
                ("text/plain", "text"),
                ("text/html", "text"),
                ("text/latex", "latex"),
                ("application/json", "json"),
            ] {
                if let Some(value) = data.get(mime) {
                    let body = match mime {
                        "text/html" => TAG.replace_all(&joined(value), "").into_owned(),
                        "application/json" => json_output(value),
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

pub fn render(path: &Path, file: File) -> String {
    let mut text = format!("# Notebook: {}\n\n", path.display());
    let nb = match serde_json::from_reader::<_, Value>(BufReader::new(file)) {
        Ok(nb) if nb.is_object() => nb,
        Ok(_) => return format!("{text}[Error parsing notebook: Invalid notebook format]\n\n"),
        Err(err) => return format!("{text}[Error parsing notebook: {err}]\n\n"),
    };
    let meta = &nb["metadata"];
    let kernel = &meta["kernelspec"];
    let lang = nonblank(&kernel["language"])
        .or_else(|| nonblank(&meta["language_info"]["name"]))
        .unwrap_or("python")
        .to_lowercase();
    let display = nonblank(&kernel["display_name"])
        .or_else(|| nonblank(&kernel["name"]))
        .unwrap_or("Unknown");
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
    write!(text, "## Notebook Metadata\n\n- Kernel: {display}\n- Language: {lang}\n- Format: {format}\n- Cells: {total} total ({breakdown})").unwrap();

    for (index, cell) in cells.iter().enumerate() {
        let source = joined(&cell["source"]);
        let source = source.trim_end();
        if source.is_empty() {
            continue;
        }
        let number = index + 1;
        let (heading, language) = match cell["cell_type"].as_str() {
            Some("markdown") => (format!("Markdown cell {number}"), "markdown"),
            Some("raw") => (
                format!("Raw cell {number}"),
                nonblank(&cell["metadata"]["format"]).unwrap_or("text"),
            ),
            Some("code") => {
                let execution = match &cell["execution_count"] {
                    Value::Null => "not executed".into(),
                    n => format!("execution: {n}"),
                };
                (format!("Code cell {number} [{execution}]"), lang.as_str())
            }
            _ => continue,
        };
        write!(text, "\n\n## {heading}\n\n{}", fenced(source, language)).unwrap();
        if cell["cell_type"] == "code"
            && let Some(outputs) = cell["outputs"].as_array()
        {
            for block in outputs.iter().filter_map(render_output) {
                write!(text, "\n\n### Output\n\n{block}").unwrap();
            }
        }
    }
    text.push_str("\n\n");
    text
}
