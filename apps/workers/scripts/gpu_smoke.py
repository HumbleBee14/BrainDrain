"""Run a minimal end-to-end SFT on the deployed Modal training app.

A real (tiny) training run through the same deployed function production uses:
dataset from object storage, adapter written back, metrics returned. Exercises
the image, the mounts, the secrets, and the training core — the gaps unit
tests cannot see.

Usage:
    python scripts/gpu_smoke.py --dataset-path <s3 key> [--base-model M] [--gpu T4]

Exits non-zero unless training completes and reports an adapter.
"""

import argparse
import json
import sys

import modal


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset-path", required=True)
    parser.add_argument("--base-model", default="Qwen/Qwen3-0.6B")
    parser.add_argument("--gpu", default="T4")
    parser.add_argument("--app-name", default="platform-training")
    parser.add_argument("--tenant-id", default="e2e00000-aaaa-4bbb-8ccc-000000000001")
    parser.add_argument("--job-id", default="c1000000-0000-4000-8000-000000000001")
    args = parser.parse_args()

    payload = {
        "input": {
            "tenant_id": args.tenant_id,
            "training_job_id": args.job_id,
            "dataset_path": args.dataset_path,
            "base_model": args.base_model,
            "method": "qlora",
            "mode": "quick",
            "hyperparams": {"num_train_epochs": 1, "max_seq_length": 512, "r": 8},
            "gpu_class": None,
        },
        "llm_config": {
            "api_base_url": "",
            "api_key": "",
            "model": "",
            "max_tokens": 512,
            "is_custom": False,
        },
    }

    fn = modal.Function.from_name(args.app_name, "train").with_options(gpu=args.gpu)
    result = fn.remote(payload)
    print(json.dumps(result, indent=2, default=str))

    if not result.get("adapter_path"):
        print("::error::smoke training returned no adapter_path", file=sys.stderr)
        return 1
    print(f"OK: adapter at {result['adapter_path']} ({result.get('adapter_size_bytes')} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
