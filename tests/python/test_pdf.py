from unittest.mock import MagicMock, patch

from focal.pdf import pdf_to_llm_text


def test_pdf_to_llm_text_success():
    with patch("pdfplumber.open") as mock_open:
        mock_pdf = MagicMock()
        mock_page1 = MagicMock()
        mock_page1.extract_text.return_value = "Page 1 text"
        mock_page2 = MagicMock()
        mock_page2.extract_text.return_value = "Page 2 text"
        mock_pdf.pages = [mock_page1, mock_page2]
        mock_open.return_value.__enter__.return_value = mock_pdf

        result = pdf_to_llm_text("sample.pdf")

        assert "# PDF: sample.pdf" in result
        assert "Page 1 text" in result
        assert "Page 2 text" in result


def test_pdf_to_llm_text_empty():
    with patch("pdfplumber.open") as mock_open:
        mock_pdf = MagicMock()
        mock_page = MagicMock()
        mock_page.extract_text.return_value = "   "
        mock_pdf.pages = [mock_page]
        mock_open.return_value.__enter__.return_value = mock_pdf

        result = pdf_to_llm_text("blank.pdf")

        assert "# PDF: blank.pdf" in result
        assert "[PDF contains no extractable text]" in result


def test_pdf_to_llm_text_page_truncation():
    with patch("pdfplumber.open") as mock_open:
        mock_pdf = MagicMock()
        pages = []
        for i in range(60):
            page = MagicMock()
            page.extract_text.return_value = f"Content page {i}"
            pages.append(page)
        mock_pdf.pages = pages
        mock_open.return_value.__enter__.return_value = mock_pdf

        result = pdf_to_llm_text("long.pdf")

        assert "...[PDF truncated: max 50 pages reached]" in result


def test_pdf_to_llm_text_error():
    with patch("pdfplumber.open", side_effect=Exception("Corrupted PDF")):
        result = pdf_to_llm_text("corrupted.pdf")

        assert "# PDF: corrupted.pdf" in result
        assert "[Error parsing PDF: Corrupted PDF]" in result
