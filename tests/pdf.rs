//! PDF extraction contracts.

mod common;

use common::*;
use std::fs;
use std::path::Path;
use std::process::Output;

/// Returns a PDF with one page per entry: text drawn in Helvetica, or a rectangle for None.
fn build_pdf(pages: &[Option<&str>]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let font_id = 3;
    let mut kids = Vec::new();
    for content in pages {
        let stream: Vec<u8> = match content {
            None => b"0 0 1 rg 72 72 200 200 re f".to_vec(),
            Some(content) => {
                let escaped = content
                    .replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)");
                assert!(escaped.is_ascii(), "fixture text must be Latin-1 ASCII");
                format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET").into_bytes()
            }
        };
        let mut object = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        object.extend_from_slice(&stream);
        object.extend_from_slice(b"\nendstream");
        objects.push(object);
        let content_id = objects.len();
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        kids.push(objects.len());
    }
    let refs = kids
        .iter()
        .map(|kid| format!("{kid} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects[1] = format!("<< /Type /Pages /Kids [{refs}] /Count {} >>", kids.len()).into_bytes();
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn run(cwd: &Path, args: &[&str]) -> Output {
    let home = cwd.join("home");
    fs::create_dir_all(&home).unwrap();
    let env = env(&[
        ("PATH", ""),
        ("HOME", home.to_str().unwrap()),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("FOCAL_OMIT_WORKSPACE", "1"),
    ]);
    let mut full = vec!["files"];
    full.extend_from_slice(args);
    focal(cwd, &env, &full, None)
}

#[test]
fn text_pages() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "doc.pdf",
        build_pdf(&[Some("Hello focal"), Some("Second (page)")]),
    );
    let result = run(tmp.path(), &["doc.pdf"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    assert_eq!(
        text(&result.stdout),
        "<context>\n# PDF: doc.pdf\n\nHello focal\n\nSecond (page)\n\n\n</context>\n"
    );
    let err = text(&result.stderr);
    assert!(err.starts_with("added: doc.pdf\n"), "{err}");
}

#[test]
fn page_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let texts: Vec<String> = (1..=60).map(|n| format!("Page number {n}")).collect();
    let pages: Vec<Option<&str>> = texts.iter().map(|t| Some(t.as_str())).collect();
    write(tmp.path(), "long.pdf", build_pdf(&pages));
    let result = run(tmp.path(), &["long.pdf"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(out.contains("Page number 50\n"), "{out}");
    assert!(!out.contains("Page number 51"), "{out}");
    assert!(
        out.contains("\n...[PDF truncated: max 50 pages reached]"),
        "{out}"
    );
}

#[test]
fn empty_pdf() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "empty.pdf", "");
    let result = run(tmp.path(), &["empty.pdf"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    assert!(text(&result.stdout).contains("# PDF: empty.pdf\n\n[empty file]\n\n"));
}

/// Runs `files <name> keep.txt` with `data` in `name`; the PDF failure must not stop the
/// other file. Returns (stdout, stderr).
fn failure(tmp: &Path, name: &str, data: &[u8]) -> (String, String) {
    write(tmp, name, data);
    write(tmp, "keep.txt", "still here\n");
    let result = run(tmp, &[name, "keep.txt"]);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(out.contains("# keep.txt\n"), "{out}");
    (out, text(&result.stderr))
}

#[test]
fn encrypted_pdf_explains_why() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/encrypted.pdf");
    let (out, err) = failure(tmp.path(), "locked.pdf", &fs::read(fixture).unwrap());
    assert!(
        out.contains("# PDF: locked.pdf\n\n[Could not extract PDF text: the PDF is encrypted"),
        "{out}"
    );
    assert!(
        err.contains("warning: could not extract text from locked.pdf: the PDF is encrypted"),
        "{err}"
    );
    assert!(err.contains("hint: decrypt it first with qpdf"), "{err}");
    assert!(
        err.contains("failed: (no text extracted): locked.pdf\n"),
        "{err}"
    );
    assert!(
        err.ends_with("Added fully: 1 files\nFailed (no text extracted): 1 files\n"),
        "{err}"
    );
}

#[test]
fn drawing_only_pdf_explains_why() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, err) = failure(tmp.path(), "art.pdf", &build_pdf(&[None]));
    assert!(
        out.contains("[Could not extract PDF text: the PDF has no text layer;"),
        "{out}"
    );
    assert!(err.contains("OCR is not supported"), "{err}");
    assert!(err.contains("hint: run OCR on it first"), "{err}");
}

fn check_truncated_pdf(cut: usize) {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = build_pdf(&[Some("Hello focal")]);
    let (out, err) = failure(tmp.path(), "cut.pdf", &pdf[..cut]);
    assert!(
        out.contains("[Could not extract PDF text: the PDF is damaged or truncated ("),
        "cut {cut}: {out}"
    );
    assert!(
        err.contains("hint: re-export or re-download the PDF"),
        "cut {cut}: {err}"
    );
}

#[test]
fn truncated_pdf_explains_why_40() {
    check_truncated_pdf(40);
}

#[test]
fn truncated_pdf_explains_why_300() {
    check_truncated_pdf(300);
}

#[test]
fn non_pdf_explains_why() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, err) = failure(tmp.path(), "fake.pdf", b"plain text with a pdf name\n");
    assert!(
        out.contains("[Could not extract PDF text: the file is not a PDF (no %PDF header)]"),
        "{out}"
    );
    assert!(
        err.contains("warning: could not extract text from fake.pdf: the file is not a PDF"),
        "{err}"
    );
}
