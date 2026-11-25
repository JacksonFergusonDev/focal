use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
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

    pub fn matches(&self, name: &str) -> bool {
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
    /// A document (such as a PDF) whose text could not be extracted; the reason is inline.
    Failed,
}

pub struct Section {
    pub markdown: String,
    pub outcome: Outcome,
}

/// A path for headings and progress lines, with control characters such as newlines and
/// ESC written as `\ooo` escapes (as `tree` does) so they cannot break the payload's
/// structure or restyle the terminal.
pub fn shown(path: &Path) -> String {
    let text = path.to_string_lossy();
    if !text.chars().any(char::is_control) {
        return text.into_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            out.push_str(&format!("\\{:03o}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Like [`format`], but a file that cannot be read becomes a warning and an inline note
/// instead of aborting the whole payload.
pub fn section(path: &Path, noise: &Noise) -> Section {
    format(path, noise).unwrap_or_else(|error| {
        eprintln!("warning: could not read {}: {error}", shown(path));
        Section {
            markdown: format!("# {}\n[Could not read file: {error}]\n\n", shown(path)),
            outcome: Outcome::Failed,
        }
    })
}

pub fn format(path: &Path, noise: &Noise) -> io::Result<Section> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let notebook = ext.eq_ignore_ascii_case("ipynb");
    let pdf = ext.eq_ignore_ascii_case("pdf");
    if size == 0 && (notebook || pdf) {
        return Ok(Section {
            markdown: "[empty file]\n\n".into(),
            outcome: Outcome::Empty,
        });
    }
    if notebook {
        return Ok(Section {
            markdown: crate::notebook::render(path, file),
            outcome: Outcome::Text,
        });
    }
    if pdf {
        let mut bytes = Vec::with_capacity(size as usize);
        BufReader::new(file).read_to_end(&mut bytes)?;
        let (markdown, extracted) = crate::pdf::render(path, &bytes);
        return Ok(Section {
            markdown,
            outcome: if extracted {
                Outcome::Text
            } else {
                Outcome::Failed
            },
        });
    }
    let (content, outcome) = if size == 0 {
        ("[empty file]".to_owned(), Outcome::Empty)
    } else if noise.matches(&name) {
        (omitted(path, size, "asset/noise"), Outcome::Skipped)
    } else {
        read_text(file, path, size)?
    };
    Ok(Section {
        markdown: format!("# {}\n{}\n\n", shown(path), fenced(&content, ext)),
        outcome,
    })
}

pub fn fenced(content: &str, language: &str) -> String {
    let longest = content
        .as_bytes()
        .split(|&byte| byte != b'`')
        .map(<[u8]>::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(3.max(longest + 1));
    format!("{fence}{language}\n{content}\n{fence}")
}

fn omitted(path: &Path, size: u64, kind: &str) -> String {
    format!(
        "[{kind} file omitted: {}]\nSize: {size} {}\nMetadata: {kind}",
        shown(path),
        if size == 1 { "byte" } else { "bytes" }
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

/// A short content-type description from magic bytes, standing in for `file -b`.
pub fn describe(path: &Path) -> String {
    let mut head = Vec::with_capacity(512);
    if let Ok(file) = File::open(path) {
        let _ = file.take(512).read_to_end(&mut head);
    }
    let magic: [(&[u8], &str); 12] = [
        (b"\x89PNG\r\n\x1a\n", "PNG image data"),
        (b"\xff\xd8\xff", "JPEG image data"),
        (b"GIF8", "GIF image data"),
        (b"%PDF-", "PDF document"),
        (b"PK\x03\x04", "Zip archive data"),
        (b"\x1f\x8b", "gzip compressed data"),
        (b"BZh", "bzip2 compressed data"),
        (b"\xfd7zXZ\x00", "XZ compressed data"),
        (b"SQLite format 3\x00", "SQLite 3.x database"),
        (b"\x7fELF", "ELF executable"),
        (b"\xcf\xfa\xed\xfe", "Mach-O 64-bit executable"),
        (b"\x93NUMPY", "NumPy array data"),
    ];
    if let Some((_, name)) = magic.iter().find(|(prefix, _)| head.starts_with(prefix)) {
        return (*name).into();
    }
    if head.is_empty() {
        return "empty".into();
    }
    if head.contains(&0) {
        return "data".into();
    }
    if head.is_ascii() {
        "ASCII text".into()
    } else if std::str::from_utf8(&head).is_ok() {
        "Unicode text, UTF-8 text".into()
    } else {
        "data".into()
    }
}
