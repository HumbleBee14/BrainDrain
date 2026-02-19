# ── Stage 1: Build ──
FROM python:3.11-slim AS builder

RUN pip install --no-cache-dir uv

WORKDIR /app

COPY pyproject.toml ./

RUN uv sync --no-dev

# ── Stage 2: Runtime ──
FROM python:3.11-slim AS runtime

RUN useradd -r -s /bin/false platform

WORKDIR /app

COPY --from=builder /app/.venv /app/.venv
COPY src/ src/

USER platform

ENV PATH="/app/.venv/bin:$PATH"
ENV PYTHONUNBUFFERED=1

CMD ["python", "-m", "src.worker"]
