//! PDF text extraction with pdf-extract, with the legacy page and character limits and
//! a specific, actionable reason whenever extraction fails.

use pdf_extract::encryption::DecryptionError;
use pdf_extract::{Document, Error, OutputError, PlainTextOutput, output_doc_page};
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Once;

const MAX_PAGES: usize = 50;
const MAX_OUTPUT_CHARS: usize = 50_000;
const DECRYPT_HINT: &str = "decrypt it first with qpdf: `qpdf --decrypt in.pdf out.pdf` if it opens without a password, or add `--password=<password>` if it needs one";

/// Why a PDF yielded no text.
#[derive(Debug, PartialEq)]
pub enum Failure {
    NotPdf,
    Damaged(String),
    PasswordRequired,
    UnsupportedEncryption(String),
    EncryptedUnreadable,
    UnsupportedFeature(String),
    Crashed,
    NoText,
}

impl Failure {
    pub fn reason(&self) -> String {
        match self {
            Self::NotPdf => "the file is not a PDF (no %PDF header)".into(),
            Self::Damaged(detail) => format!("the PDF is damaged or truncated ({detail})"),
            Self::PasswordRequired => "the PDF is encrypted and requires a password to open".into(),
            Self::UnsupportedEncryption(detail) => {
                format!("the PDF is encrypted, and the extractor cannot decrypt it ({detail})")
            }
            Self::EncryptedUnreadable => {
                "the PDF is encrypted, and the extractor could not decrypt its text".into()
            }
            Self::UnsupportedFeature(detail) => {
                format!("the PDF uses a feature the extractor does not support ({detail})")
            }
            Self::Crashed => "the PDF parser crashed on this file's structure".into(),
            Self::NoText => {
                "the PDF has no text layer; it may contain only scanned images or drawings, and OCR is not supported".into()
            }
        }
    }

    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::PasswordRequired => Some(
                "decrypt it first with qpdf: `qpdf --password=<password> --decrypt in.pdf out.pdf`",
            ),
            Self::UnsupportedEncryption(_) | Self::EncryptedUnreadable => Some(DECRYPT_HINT),
            Self::Damaged(_) => Some("re-export or re-download the PDF"),
            Self::NoText => Some("run OCR on it first, for example: `ocrmypdf in.pdf out.pdf`"),
            _ => None,
        }
    }
}

fn classify(error: &Error) -> Failure {
    match error {
        Error::Decryption(DecryptionError::IncorrectPassword) => Failure::PasswordRequired,
        Error::Decryption(detail) => Failure::UnsupportedEncryption(detail.to_string()),
        Error::Unimplemented(detail) => Failure::UnsupportedFeature((*detail).into()),
        other => Failure::Damaged(other.to_string()),
    }
}

fn classify_output(error: &OutputError) -> Failure {
    match error {
        OutputError::PdfError(error) => classify(error),
        other => Failure::Damaged(other.to_string()),
    }
}

thread_local! {
    /// Set while this thread runs PDF parsing whose panics are expected and reported.
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Installs, once per process, a panic hook that stays silent for panics inside [`guarded`]
/// and defers to the default hook otherwise. Swapping hooks per call would race when the
/// picker's preview worker and the main thread parse PDFs at the same time.
fn install_quiet_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                default(info);
            }
        }));
    });
}

/// Runs `f`, converting a panic into `Failure::Crashed` without printing the panic message.
fn guarded<T>(f: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
    install_quiet_hook();
    let outer = QUIET.with(|quiet| quiet.replace(true));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|quiet| quiet.set(outer));
    result.unwrap_or(Err(Failure::Crashed))
}

/// Replaces typographic ligatures, which pdf-extract leaves in place, with plain letters.
fn unligature(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            _ => out.push(ch),
        }
    }
    out
}

/// Extracted text, or the reason none could be extracted. Pages that fail individually
/// become inline notes as long as at least one page yields text.
pub fn extract(bytes: &[u8]) -> Result<String, Failure> {
    let header = &bytes[..bytes.len().min(1024)];
    if !header.windows(5).any(|window| window == b"%PDF-") {
        return Err(Failure::NotPdf);
    }
    guarded(|| {
        let mut doc = Document::load_mem(bytes).map_err(|e| classify(&e))?;
        let encrypted = doc.is_encrypted();
        if encrypted {
            doc.decrypt("").map_err(|e| classify(&e))?;
        }
        let page_numbers: Vec<u32> = doc.get_pages().into_keys().collect();
        let mut pages = Vec::new();
        let mut first_failure = None;
        for (index, number) in page_numbers.iter().enumerate() {
            if index >= MAX_PAGES {
                pages.push(format!(
                    "\n...[PDF truncated: max {MAX_PAGES} pages reached]"
                ));
                break;
            }
            let mut text = String::new();
            let page = guarded(|| {
                let mut output = PlainTextOutput::new(&mut text);
                output_doc_page(&doc, &mut output, *number).map_err(|e| classify_output(&e))
            });
            match page {
                Ok(()) => {
                    let text = unligature(text.trim());
                    if !text.is_empty() {
                        pages.push(text);
                    }
                }
                Err(failure) => {
                    pages.push(format!(
                        "[page {number} could not be extracted: {}]",
                        failure.reason()
                    ));
                    first_failure.get_or_insert(failure);
                }
            }
        }
        let extracted = pages
            .iter()
            .any(|page| !page.starts_with('[') && !page.starts_with("\n..."));
        if !extracted {
            return Err(match first_failure {
                Some(failure) => failure,
                None if encrypted => Failure::EncryptedUnreadable,
                None => Failure::NoText,
            });
        }
        let mut content = pages.join("\n\n");
        if content.chars().count() > MAX_OUTPUT_CHARS {
            content = content.chars().take(MAX_OUTPUT_CHARS).collect();
            content.push_str(&format!(
                "\n...[PDF truncated: exceeded {MAX_OUTPUT_CHARS} characters]"
            ));
        }
        Ok(content)
    })
}

/// Renders the PDF section. Failures are explained inline for the reader of the payload
/// and as a warning with a hint on stderr for the person running the command.
pub fn render(path: &Path, bytes: &[u8]) -> (String, bool) {
    match extract(bytes) {
        Ok(content) => (
            format!("# PDF: {}\n\n{content}\n\n", crate::file::shown(path)),
            true,
        ),
        Err(failure) => {
            let reason = failure.reason();
            eprintln!(
                "warning: could not extract text from {}: {reason}",
                crate::file::shown(path)
            );
            if let Some(hint) = failure.hint() {
                eprintln!("  hint: {hint}");
            }
            (
                format!(
                    "# PDF: {}\n\n[Could not extract PDF text: {reason}]\n\n",
                    crate::file::shown(path)
                ),
                false,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_panics_become_crashes_on_any_thread() {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..50)
                        .map(|_| guarded::<()>(|| panic!("expected test panic")))
                        .all(|result| result == Err(Failure::Crashed))
                })
            })
            .collect();
        assert!(workers.into_iter().all(|worker| worker.join().unwrap()));
        assert_eq!(guarded(|| Ok(3)), Ok(3));
    }

    #[test]
    fn ligatures_become_letters() {
        assert_eq!(
            unligature("\u{FB01}le e\u{FB00}ort \u{FB04}"),
            "file effort ffl"
        );
    }

    #[test]
    fn rejects_files_without_a_pdf_header() {
        assert_eq!(extract(b"not a pdf"), Err(Failure::NotPdf));
    }

    #[test]
    fn classifies_decryption_errors() {
        let wrong = Error::Decryption(DecryptionError::IncorrectPassword);
        assert_eq!(classify(&wrong), Failure::PasswordRequired);
        let unsupported = Error::Decryption(DecryptionError::InvalidKeyLength);
        assert!(
            classify(&unsupported)
                .reason()
                .starts_with("the PDF is encrypted")
        );
    }
}
