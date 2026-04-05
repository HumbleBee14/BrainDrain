# Inference Backend Abstraction

**PR:** #18  
**Problem:** Different GPU serving engines (vLLM, TGI, SGLang) have different
APIs for loading LoRA adapters and selecting them at inference time. Without
abstraction, switching engines means rewriting deploy and inference code.

## How it works

The `InferenceBackend` trait abstracts three things:
1. **Loading** a LoRA adapter onto the serving engine
2. **Unloading** an adapter
3. **Building** the inference request with the adapter reference

Each backend knows its own wire format. The rest of the platform just calls
`backend.load_adapter()` and `backend.build_inference_body()`.

## Supported backends

| Backend | Dynamic LoRA? | Adapter selection | Type string |
|---|---|---|---|
| **vLLM** | Yes (REST API) | `model` field | `vllm` |
| **SGLang** | Yes (REST API) | `model` field | `sglang` |
| **TGI** | No (startup only) | `parameters.adapter_id` | `tgi` |

## Configuration

Set via environment:
```bash
INFERENCE_BACKEND_TYPE=vllm          # or tgi, sglang
INFERENCE_SERVER_URL=http://vllm:8000
```

## Files

- `crates/api/src/services/inference_backend.rs` — Trait + 3 implementations
- `crates/api/src/services/deployment_service.rs` — Uses the trait
- `crates/api/src/routes/inference.rs` — Uses the trait
