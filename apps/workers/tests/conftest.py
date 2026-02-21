"""Shared test fixtures for worker activity tests."""

import pytest


@pytest.fixture
def sample_pdf_bytes():
    """Minimal valid PDF bytes for testing."""
    # A minimal valid PDF (empty page)
    return (
        b"%PDF-1.0\n"
        b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n"
        b"2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n"
        b"3 0 obj<</Type/Page/MediaBox[0 0 612 792]/Parent 2 0 R>>endobj\n"
        b"xref\n0 4\n"
        b"0000000000 65535 f \n"
        b"0000000009 00000 n \n"
        b"0000000058 00000 n \n"
        b"0000000115 00000 n \n"
        b"trailer<</Size 4/Root 1 0 R>>\n"
        b"startxref\n190\n%%EOF"
    )


@pytest.fixture
def sample_text_bytes():
    """Plain text bytes for testing."""
    return b"This is a test document.\n\nIt has multiple paragraphs.\n\nThird paragraph here."


@pytest.fixture
def sample_html_bytes():
    """HTML bytes for testing."""
    return b"<html><body><h1>Title</h1><p>Content paragraph.</p></body></html>"


@pytest.fixture
def sample_csv_bytes():
    """CSV bytes for testing."""
    return b"name,age,city\nAlice,30,NYC\nBob,25,LA\n"


@pytest.fixture
def sample_markdown_bytes():
    """Markdown bytes for testing."""
    return b"# Heading\n\nSome paragraph text.\n\n## Subheading\n\nMore text."
