"""PDF extraction backend — swap the library without touching PdfParser.

Protocol: PdfBackend
  extract_pages(raw_bytes) -> list[dict]
  Each dict: {page_num: int, text: str, sections: list[dict]}

Built-in backends:
  "pymupdf"  — PyMuPDF / fitz (default, fast, no extra install)
  "docling"  — Docling (richer structure, install with extras [pdf-ml])

Register custom backends with register().
"""

from typing import Protocol


class PdfBackend(Protocol):
    """Protocol for PDF extraction backends."""

    def extract_pages(self, raw_bytes: bytes) -> list[dict]:
        """Extract pages from raw PDF bytes.

        Returns a list of page dicts:
            {page_num: int, text: str, sections: list[dict]}
        """
        ...


# -- Implementations --


class PyMuPdfBackend:
    """PDF extraction using PyMuPDF (fitz). Default backend."""

    def extract_pages(self, raw_bytes: bytes) -> list[dict]:
        import fitz  # PyMuPDF

        pages = []
        doc = fitz.open(stream=raw_bytes, filetype="pdf")

        for page_num, page in enumerate(doc, start=1):
            blocks = page.get_text("dict", flags=fitz.TEXT_PRESERVE_WHITESPACE)["blocks"]
            sections = []
            page_text_parts = []

            for block in blocks:
                if block["type"] == 0:  # text block
                    for line in block.get("lines", []):
                        text = "".join(span["text"] for span in line.get("spans", []))
                        if not text.strip():
                            continue

                        max_size = max(
                            (span.get("size", 12) for span in line.get("spans", [])),
                            default=12,
                        )
                        section_type = "heading" if max_size > 14 else "paragraph"
                        sections.append({"type": section_type, "content": text.strip()})
                        page_text_parts.append(text.strip())

            pages.append(
                {
                    "page_num": page_num,
                    "text": "\n".join(page_text_parts),
                    "sections": sections,
                }
            )

        doc.close()
        return pages


class DoclingBackend:
    """PDF extraction using Docling. Install with: pip install braindrain-workers[pdf-ml]

    Produces richer structure (tables, figures, reading order) at the cost of
    higher CPU/memory usage and a larger install.
    """

    def extract_pages(self, raw_bytes: bytes) -> list[dict]:
        try:
            import io

            from docling.datamodel.base_models import InputFormat
            from docling.datamodel.pipeline_options import PdfPipelineOptions
            from docling.document_converter import DocumentConverter, PdfFormatOption
        except ImportError as e:
            raise ImportError(
                "Docling is not installed. Run: pip install braindrain-workers[pdf-ml]"
            ) from e

        pipeline_options = PdfPipelineOptions()
        pipeline_options.do_ocr = False  # disable OCR for speed; enable via subclass if needed
        converter = DocumentConverter(
            format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=pipeline_options)}
        )

        result = converter.convert(source=io.BytesIO(raw_bytes), raises_on_error=True)
        doc = result.document

        # Group exported items by page number
        page_buckets: dict[int, dict] = {}

        for item, _level in doc.iterate_items():
            page_no = getattr(item.prov[0], "page_no", 1) if item.prov else 1
            if page_no not in page_buckets:
                page_buckets[page_no] = {"page_num": page_no, "text": "", "sections": []}

            text = getattr(item, "text", None) or ""
            if not text.strip():
                continue

            label = getattr(item, "label", "paragraph")
            section_type = "heading" if "heading" in str(label).lower() else "paragraph"
            page_buckets[page_no]["sections"].append({"type": section_type, "content": text})
            page_buckets[page_no]["text"] += ("\n" if page_buckets[page_no]["text"] else "") + text

        return [page_buckets[k] for k in sorted(page_buckets)]


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "pymupdf": PyMuPdfBackend,
    "docling": DoclingBackend,
}


def register(name: str, cls: type) -> None:
    """Register a custom PdfBackend implementation."""
    _REGISTRY[name] = cls


def get(name: str) -> PdfBackend:
    """Instantiate the named PdfBackend.

    Raises ValueError listing available backends if name is unknown.
    """
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown pdf_backend '{name}'. Available: {available}")
    return cls()
