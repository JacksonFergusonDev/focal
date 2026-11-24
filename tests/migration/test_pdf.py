"""PDF extraction contracts shared by the legacy dispatcher and the Rust preview."""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ
FIXTURES = Path(__file__).parent / "fixtures"


def build_pdf(pages: list[str | None]) -> bytes:
    """Return a PDF with one page per entry: text drawn in Helvetica, or a rectangle for None."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b""]
    kids = []
    font_id = 3
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    for content in pages:
        if content is None:
            stream = b"0 0 1 rg 72 72 200 200 re f"
        else:
            escaped = (
                content.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
            )
            stream = (
                b"BT /F1 12 Tf 72 720 Td (" + escaped.encode("latin-1") + b") Tj ET"
            )
        objects.append(
            b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream)
        )
        content_id = len(objects)
        objects.append(
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << /F1 %d 0 R >> >> /Contents %d 0 R >>"
            % (font_id, content_id)
        )
        kids.append(len(objects))
    refs = b" ".join(b"%d 0 R" % kid for kid in kids)
    objects[1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (refs, len(kids))
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n%s\nendobj\n" % (number, body)
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        xref,
    )
    return bytes(out)


@pytest.fixture
def env(tmp_path):
    home = tmp_path / "home"
    home.mkdir()
    return {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "LC_ALL": "C",
        "NO_COLOR": "1",
        "FOCAL_OMIT_WORKSPACE": "1",
    }


def run(cwd, env, *args):
    if os.environ.get("FOCAL_TEST_EMPTY_PATH"):
        env = {**env, "PATH": ""}
    return subprocess.run(
        [CLI, "files", *args], cwd=cwd, env=env, capture_output=True, check=False
    )


def test_text_pages(tmp_path, env):
    (tmp_path / "doc.pdf").write_bytes(build_pdf(["Hello focal", "Second (page)"]))
    result = run(tmp_path, env, "doc.pdf")
    assert result.returncode == 0, result.stderr
    assert result.stdout == (
        b"<context>\n# PDF: doc.pdf\n\nHello focal\n\nSecond (page)\n\n\n</context>\n"
    )
    assert result.stderr.startswith(b"added: doc.pdf\n")


def test_page_limit(tmp_path, env):
    pages: list[str | None] = [f"Page number {n}" for n in range(1, 61)]
    (tmp_path / "long.pdf").write_bytes(build_pdf(pages))
    result = run(tmp_path, env, "long.pdf")
    assert result.returncode == 0, result.stderr
    out = result.stdout.decode()
    assert "Page number 50\n" in out
    assert "Page number 51" not in out
    assert "\n...[PDF truncated: max 50 pages reached]" in out


def test_empty_pdf(tmp_path, env):
    (tmp_path / "empty.pdf").touch()
    result = run(tmp_path, env, "empty.pdf")
    assert result.returncode == 0, result.stderr
    assert b"[empty file]" in result.stdout


def failure(tmp_path, env, name, data):
    (tmp_path / name).write_bytes(data)
    (tmp_path / "keep.txt").write_text("still here\n")
    result = run(tmp_path, env, name, "keep.txt")
    assert result.returncode == 0, result.stderr
    assert b"# keep.txt\n" in result.stdout
    return result.stdout.decode(), result.stderr.decode()


@pytest.mark.xfail(LEGACY, reason="legacy reads owner-restricted PDFs", strict=True)
def test_encrypted_pdf_explains_why(tmp_path, env):
    shutil.copy(FIXTURES / "encrypted.pdf", tmp_path / "locked.pdf")
    out, err = failure(
        tmp_path, env, "locked.pdf", (FIXTURES / "encrypted.pdf").read_bytes()
    )
    assert (
        "# PDF: locked.pdf\n\n[Could not extract PDF text: the PDF is encrypted" in out
    )
    assert (
        "warning: could not extract text from locked.pdf: the PDF is encrypted" in err
    )
    assert "hint: decrypt it first with qpdf" in err
    assert "failed: (no text extracted): locked.pdf\n" in err
    assert err.endswith("Added fully: 1 files\nFailed (no text extracted): 1 files\n")


@pytest.mark.xfail(
    LEGACY, reason="legacy reports only that no text was found", strict=True
)
def test_drawing_only_pdf_explains_why(tmp_path, env):
    out, err = failure(tmp_path, env, "art.pdf", build_pdf([None]))
    assert "[Could not extract PDF text: the PDF has no text layer;" in out
    assert "OCR is not supported" in err
    assert "hint: run OCR on it first" in err


@pytest.mark.xfail(LEGACY, reason="legacy reports a raw parser error", strict=True)
@pytest.mark.parametrize("cut", [40, 300])
def test_truncated_pdf_explains_why(tmp_path, env, cut):
    out, err = failure(tmp_path, env, "cut.pdf", build_pdf(["Hello focal"])[:cut])
    assert "[Could not extract PDF text: the PDF is damaged or truncated (" in out
    assert "hint: re-export or re-download the PDF" in err


@pytest.mark.xfail(LEGACY, reason="legacy reports a raw parser error", strict=True)
def test_non_pdf_explains_why(tmp_path, env):
    out, err = failure(tmp_path, env, "fake.pdf", b"plain text with a pdf name\n")
    assert "[Could not extract PDF text: the file is not a PDF (no %PDF header)]" in out
    assert "warning: could not extract text from fake.pdf: the file is not a PDF" in err
