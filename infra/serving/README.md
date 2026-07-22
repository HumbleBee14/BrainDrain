# Serving: vLLM with S3 LoRA resolver

The platform trains LoRA adapters and stores them in object storage (S3 / MinIO
/ Cloudflare R2) under a per-model key prefix (the `adapter_path` on the model
row, e.g. `adapters/<tenant>/<model>/`). To serve a deployed model, vLLM must
load that adapter — but vLLM only reads adapters from its **own local
filesystem**.

This image bridges that gap with a **LoRA resolver plugin**
([`vllm_s3_lora_resolver`](./vllm_s3_lora_resolver/)). When an inference request
names an adapter vLLM hasn't loaded yet, the plugin downloads it from the bucket
to a local cache and hands it to vLLM. No shared volume between the control
plane and the GPU box, and no adapter baked into the image.

## How the pieces fit

1. Training uploads the adapter to `s3://<bucket>/adapters/<tenant>/<model>/`.
2. The control plane's `deploy` sets the model's `adapter_ref` to that S3 key
   prefix and sends a **warmup** inference (`model = <adapter_ref>`) so any
   resolve/load failure surfaces at deploy time, not on the first user request.
3. vLLM sees an unknown adapter name → the S3 resolver fetches it → vLLM loads
   it. Subsequent requests hit the local cache.

## Required environment variables

| Var | Purpose |
|---|---|
| `S3_LORA_BUCKET` (or `S3_BUCKET`) | Bucket holding adapters |
| `VLLM_LORA_RESOLVER_CACHE_DIR` | Local dir for downloaded adapters (default `/var/lora-cache`) |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | Object-store credentials (boto3 standard) |
| `S3_ENDPOINT_URL` (or `S3_ENDPOINT`) | Custom endpoint for MinIO/R2; omit for AWS |
| `S3_REGION` (or `AWS_REGION`) | Region (`auto` for R2) |
| `S3_LORA_STRICT_BASE_MODEL` | `true` to reject adapters whose declared base model doesn't match the served model (default `false`, warn-only) |
| `VLLM_ALLOW_RUNTIME_LORA_UPDATING` | Must be `true` (set in the image) |
| `VLLM_PLUGINS` | Must include `s3_lora_resolver` (set in the image) |

## Run

```bash
docker build -f infra/serving/Dockerfile.vllm -t platform-vllm infra/serving

docker run --gpus all -p 8000:8000 \
  -e S3_LORA_BUCKET=brain-drain \
  -e S3_ENDPOINT_URL=https://<account>.r2.cloudflarestorage.com \
  -e S3_REGION=auto \
  -e AWS_ACCESS_KEY_ID=... -e AWS_SECRET_ACCESS_KEY=... \
  platform-vllm \
  --model unsloth/Qwen2.5-0.5B-Instruct \
  --enable-lora --max-lora-rank 64 --max-loras 4
```

On Turing GPUs (T4, compute capability 7.5) add `--dtype half` — they don't
support bfloat16. Ampere and newer (A10, A100, L4, H100) run `--dtype auto`.

Register the running server as an inference instance (or point
`INFERENCE_SERVER_URL` at it for single-instance mode), then deploy a model
through the API. The control plane routes inference to the adapter by name.

The resolver is unit-tested (vLLM + boto3 stubbed):

```bash
cd apps/workers && uv run --with pytest python -m pytest \
  ../../infra/serving/vllm_s3_lora_resolver/tests -q
```
