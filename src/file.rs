use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

const MAX_LINES: usize = 1_500;
const MAX_BYTES: usize = 100_000;

#[derive(Deserialize)]
pub struct Noise {
    extensions: Vec<String>,
    files: Vec<String>,
}

impl Noise {
    pub fn load() -> Self {
        serde_json::from_str(include_str!("../lib/noise.json"))
            .expect("invalid embedded noise.json")
    }

    fn matches(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        self.files.iter().any(|file| file == name)
            || self.extensions.iter().any(|ext| {
                lower
                    .strip_suffix(ext)
                    .is_some_and(|stem| stem.ends_with('.'))
            })
    }
}

pub enum Outcome {
    Text,
    Empty,
    Skipped,
    Truncated,
}

pub struct Section {
    pub markdown: String,
    pub outcome: Outcome,
}

pub fn format(path: &Path, noise: &Noise) -> io::Result<Section> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let (content, outcome) = if size == 0 {
        ("[empty file]".to_owned(), Outcome::Empty)
    } else if ext.eq_ignore_ascii_case("ipynb") || ext.eq_ignore_ascii_case("pdf") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "document extraction is pending in the Rust preview",
        ));
    } else if noise.matches(&name) {
        (omitted(path, size, "asset/noise"), Outcome::Skipped)
    } else {
        read_text(file, path, size)?
    };
    let longest = content
        .as_bytes()
        .split(|&byte| byte != b'`')
        .map(<[u8]>::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(3.max(longest + 1));
    Ok(Section {
        markdown: format!("# {}\n{fence}{ext}\n{content}\n{fence}\n\n", path.display()),
        outcome,
    })
}

fn omitted(path: &Path, size: u64, kind: &str) -> String {
    format!(
        "[{kind} file omitted: {}]\nSize: {size} bytes\nMetadata: {kind}",
        path.display()
    )
}

fn read_text(file: File, path: &Path, size: u64) -> io::Result<(String, Outcome)> {
    let mut reader = BufReader::new(file);
    if reader.fill_buf()?.contains(&0) {
        return Ok((omitted(path, size, "binary"), Outcome::Skipped));
    }
    let mut prefix = Vec::new();
    let mut lines = 0;
    let mut line_cutoff = None;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        for (i, &byte) in chunk.iter().enumerate() {
            if byte == b'\n' {
                lines += 1;
                if lines == MAX_LINES {
                    line_cutoff = Some(prefix.len() + i + 1);
                }
            }
        }
        let keep = chunk.len().min(MAX_BYTES - prefix.len());
        prefix.extend_from_slice(&chunk[..keep]);
        let consumed = chunk.len();
        reader.consume(consumed);
    }
    let notice = if lines > MAX_LINES {
        prefix.truncate(line_cutoff.unwrap_or(MAX_BYTES));
        Some(format!("showing first {MAX_LINES} of {lines} lines"))
    } else if size > MAX_BYTES as u64 {
        Some(format!("showing first {MAX_BYTES} of {size} bytes"))
    } else {
        None
    };
    while prefix.last() == Some(&b'\n') {
        prefix.pop();
    }
    let mut content = String::from_utf8_lossy(&prefix).into_owned();
    if let Some(notice) = notice {
        content.push_str(&format!("\n\n...[file truncated: {notice}]"));
        Ok((content, Outcome::Truncated))
    } else {
        Ok((content, Outcome::Text))
    }
}
