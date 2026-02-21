"""Tests for document parsing activity -- parser logic only (no S3/DB)."""

# Import the internal parser functions directly
from src.activities.parse_document import (
    _compute_quality,
    _detect_language,
    _parse_by_type,
    _parser_name,
)


class TestParseByType:
    """Test MIME type routing to correct parsers."""

    def test_pdf_by_mime(self, sample_pdf_bytes):
        """PDF mime type routes to PDF parser."""
        # May fail on minimal PDF, but should not raise wrong-parser errors
        pages = _parse_by_type(sample_pdf_bytes, "application/pdf", "doc.pdf")
        assert isinstance(pages, list)

    def test_text_by_mime(self, sample_text_bytes):
        pages = _parse_by_type(sample_text_bytes, "text/plain", "doc.txt")
        assert len(pages) == 1
        assert "test document" in pages[0]["text"]

    def test_html_by_mime(self, sample_html_bytes):
        pages = _parse_by_type(sample_html_bytes, "text/html", "doc.html")
        assert len(pages) == 1
        assert any(s["type"] == "heading" for s in pages[0]["sections"])

    def test_csv_by_mime(self, sample_csv_bytes):
        pages = _parse_by_type(sample_csv_bytes, "text/csv", "data.csv")
        assert len(pages) == 1
        assert "table" in [s["type"] for s in pages[0]["sections"]]

    def test_markdown_by_mime(self, sample_markdown_bytes):
        pages = _parse_by_type(sample_markdown_bytes, "text/markdown", "doc.md")
        assert len(pages) == 1

    def test_unknown_mime_falls_back_to_text(self, sample_text_bytes):
        pages = _parse_by_type(sample_text_bytes, "application/octet-stream", "doc.bin")
        assert len(pages) >= 1

    def test_fallback_by_extension(self, sample_text_bytes):
        """Falls back to extension when mime type is generic."""
        pages = _parse_by_type(sample_text_bytes, "application/octet-stream", "file.txt")
        assert len(pages) >= 1


class TestDetectLanguage:
    def test_english_detection(self):
        text = "This is a sample English text that should be long enough for detection."
        assert _detect_language(text) == "en"

    def test_short_text_returns_none(self):
        assert _detect_language("hi") is None

    def test_empty_text_returns_none(self):
        assert _detect_language("") is None


class TestComputeQuality:
    def test_empty_pages_returns_zero(self):
        assert _compute_quality([], 100) == 0.0

    def test_pages_with_no_text_returns_zero(self):
        pages = [{"text": "", "sections": []}]
        assert _compute_quality(pages, 100) == 0.0

    def test_good_quality_document(self):
        pages = [
            {
                "text": "A" * 400,
                "sections": [{"type": "heading", "content": "Title"}],
            }
        ]
        quality = _compute_quality(pages, 500)
        assert 0.5 < quality <= 1.0

    def test_quality_bounded_zero_to_one(self):
        pages = [{"text": "x" * 10, "sections": []}]
        quality = _compute_quality(pages, 100)
        assert 0.0 <= quality <= 1.0


class TestParserName:
    def test_pdf_parser_name(self):
        assert _parser_name("application/pdf") == "pymupdf"

    def test_word_parser_name(self):
        assert "docx" in _parser_name(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        )

    def test_html_parser_name(self):
        assert _parser_name("text/html") == "beautifulsoup"

    def test_csv_parser_name(self):
        assert _parser_name("text/csv") == "csv"

    def test_unknown_parser_name(self):
        assert _parser_name("application/octet-stream") == "plaintext"
