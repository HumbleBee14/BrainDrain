-- Model exports (GGUF, ONNX future)
CREATE TABLE IF NOT EXISTS model_exports (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id),
    model_id        UUID NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    format          TEXT NOT NULL DEFAULT 'gguf',
    quant_type      TEXT NOT NULL DEFAULT 'Q5_K_M',
    status          TEXT NOT NULL DEFAULT 'pending',
    storage_path    TEXT,
    file_size_bytes BIGINT,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_model_exports_model ON model_exports (model_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_model_exports_tenant ON model_exports (tenant_id);
