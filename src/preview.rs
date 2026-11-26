//! Preview loading for the picker. Previews render on a worker thread that always skips to
//! the newest request, so a slow document never blocks input and a stale result never
//! replaces the preview of the item now under the cursor.

use crate::file::Noise;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const MAX_PREVIEW_LINES: usize = 1_000;
const MAX_PREVIEW_BYTES: u64 = 256 * 1024;
const NONE: usize = usize::MAX;

pub struct Previewer {
    requests: Sender<usize>,
    results: Receiver<(usize, String)>,
    latest: Arc<AtomicUsize>,
}

impl Previewer {
    pub fn new(loader: Arc<dyn Fn(usize) -> String + Send + Sync>) -> Self {
        let (requests, inbox) = mpsc::channel::<usize>();
        let (outbox, results) = mpsc::channel();
        let latest = Arc::new(AtomicUsize::new(NONE));
        let wanted = Arc::clone(&latest);
        thread::spawn(move || {
            while let Ok(mut id) = inbox.recv() {
                // Coalesce queued requests; only the newest one matters.
                while let Ok(newer) = inbox.try_recv() {
                    id = newer;
                }
                if wanted.load(Ordering::Acquire) != id {
                    continue;
                }
                let text = loader(id);
                if outbox.send((id, text)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests,
            results,
            latest,
        }
    }

    /// Asks for `id`'s preview, superseding any earlier request still waiting.
    pub fn request(&self, id: usize) {
        self.latest.store(id, Ordering::Release);
        let _ = self.requests.send(id);
    }

    pub fn try_result(&self) -> Option<(usize, String)> {
        self.results.try_recv().ok()
    }
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b}B"),
        b if b < 1024 * 1024 => format!("{:.1}K", b as f64 / 1024.0),
        b => format!("{:.1}M", b as f64 / (1024.0 * 1024.0)),
    }
}

fn metadata(path: &Path, size: u64, reason: &str) -> String {
    format!(
        "[{reason} omitted from preview]\n\nFile: {}\nType: {}\nSize: {}",
        crate::file::shown(path),
        crate::file::describe(path),
        human_size(size)
    )
}

/// Preview text for a file: numbered text, rendered notebooks and PDFs, or metadata for
/// noise and binary files. Mirrors what extraction would include.
pub fn file(path: &Path, noise: &Noise) -> String {
    let size = match fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(error) => return format!("[cannot read {}: {error}]", crate::file::shown(path)),
    };
    if size == 0 {
        return "[empty file]".into();
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext = name.rsplit_once('.').map_or("", |(_, ext)| ext);
    if ext.eq_ignore_ascii_case("ipynb") {
        return match File::open(path) {
            Ok(file) => crate::notebook::render(path, file).0,
            Err(error) => format!("[cannot read {}: {error}]", crate::file::shown(path)),
        };
    }
    if ext.eq_ignore_ascii_case("pdf") {
        return match fs::read(path) {
            Ok(bytes) => match crate::pdf::extract(&bytes) {
                Ok(text) => text,
                Err(failure) => format!("[Could not extract PDF text: {}]", failure.reason()),
            },
            Err(error) => format!("[cannot read {}: {error}]", crate::file::shown(path)),
        };
    }
    if noise.matches(&name) {
        return metadata(path, size, "Asset/noise file");
    }
    let Ok(file) = File::open(path) else {
        return format!("[cannot read {}]", crate::file::shown(path));
    };
    let mut head = Vec::new();
    if BufReader::new(file)
        .take(MAX_PREVIEW_BYTES)
        .read_to_end(&mut head)
        .is_err()
    {
        return format!("[cannot read {}]", crate::file::shown(path));
    }
    if head[..head.len().min(8192)].contains(&0) {
        return metadata(path, size, "Binary file");
    }
    let mut out = String::new();
    let lines: Vec<&[u8]> = head.split_inclusive(|&b| b == b'\n').collect();
    let width = lines.len().min(MAX_PREVIEW_LINES).to_string().len();
    for (number, line) in head.lines().take(MAX_PREVIEW_LINES).enumerate() {
        let line = line.unwrap_or_default().replace('\t', "    ");
        let clean: String = line.chars().filter(|c| !c.is_control()).collect();
        let _ = writeln!(out, "{:>width$} │ {clean}", number + 1);
    }
    if lines.len() > MAX_PREVIEW_LINES || size > MAX_PREVIEW_BYTES {
        out.push_str("…\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(previewer: &Previewer, timeout: Duration) -> Vec<(usize, String)> {
        let start = Instant::now();
        let mut got = Vec::new();
        while start.elapsed() < timeout {
            while let Some(result) = previewer.try_result() {
                got.push(result);
            }
            thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn slow_preview_does_not_block_or_replace_newer_requests() {
        let loader: Arc<dyn Fn(usize) -> String + Send + Sync> = Arc::new(|id| {
            if id == 0 {
                thread::sleep(Duration::from_millis(300));
            }
            format!("preview {id}")
        });
        let previewer = Previewer::new(loader);
        let start = Instant::now();
        previewer.request(0);
        // The caller keeps moving while item 0 is still rendering.
        for id in 1..=5 {
            previewer.request(id);
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "requests must not block"
        );
        let results = wait(&previewer, Duration::from_millis(600));
        // Item 0 may finish, but intermediate requests 1-4 are skipped and 5 arrives.
        assert!(
            results
                .iter()
                .any(|(id, text)| *id == 5 && text == "preview 5")
        );
        assert!(results.iter().all(|(id, _)| *id == 0 || *id == 5));
    }

    #[test]
    fn previews_text_with_line_numbers_and_flags_binaries() {
        let dir = std::env::temp_dir().join(format!("focal-preview-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let text = dir.join("a.rs");
        fs::write(&text, "fn main() {\n\tprintln!(\"hi\");\n}\n").unwrap();
        let binary = dir.join("blob.dat");
        fs::write(&binary, b"\x00\x01binary").unwrap();
        let noise = Noise::load();
        assert_eq!(
            file(&text, &noise),
            "1 │ fn main() {\n2 │     println!(\"hi\");\n3 │ }\n"
        );
        assert!(file(&binary, &noise).starts_with("[Binary file omitted from preview]"));
        assert!(file(&dir.join("x.png"), &noise).starts_with("[cannot read"));
        fs::write(dir.join("x.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let png = file(&dir.join("x.png"), &noise);
        assert!(png.starts_with("[Asset/noise file omitted from preview]"));
        assert!(png.contains("Type: PNG image data"));
        fs::remove_dir_all(dir).unwrap();
    }
}
