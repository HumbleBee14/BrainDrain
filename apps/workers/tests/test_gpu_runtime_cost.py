"""Tests for GPU-runtime cost accounting.

Aligned (SFT→DPO) and reasoning (SFT→GRPO) runs return the DPO/GRPO phase
runtime nested one level under a "dpo"/"grpo" key. The cost/GPU-seconds sums
must count that nested runtime, not just the flat SFT `train_runtime`, or those
runs are billed for the SFT pass only (undercharge).
"""

from src.activities.train_model import (
    _extract_training_runtime_seconds,
    _sum_gpu_runtime_seconds,
)


class TestSumGpuRuntimeSeconds:
    def test_flat_sft_runtime(self):
        m = {"sft_train_runtime": 100.0, "sft_train_loss": 0.5}
        assert _sum_gpu_runtime_seconds(m) == 100.0

    def test_includes_nested_dpo_runtime(self):
        m = {"sft_train_runtime": 100.0, "dpo": {"dpo_runtime": 40.0, "dpo_loss": 0.2}}
        assert _sum_gpu_runtime_seconds(m) == 140.0

    def test_includes_nested_grpo_runtime(self):
        m = {"sft_train_runtime": 100.0, "grpo": {"grpo_runtime": 55.0, "grpo_loss": 0.1}}
        assert _sum_gpu_runtime_seconds(m) == 155.0

    def test_iterative_nested_iteration_runtimes(self):
        m = {
            "iter_0": {"iter_0_train_runtime": 30.0},
            "iter_1": {"iter_1_train_runtime": 20.0},
            "total_iterations": 2,
            "best_eval_loss": 0.4,
        }
        assert _sum_gpu_runtime_seconds(m) == 50.0

    def test_ignores_non_runtime_keys(self):
        m = {"train_samples_per_second": 5.0, "estimated_cost": 1.23, "train_runtime": 10.0}
        assert _sum_gpu_runtime_seconds(m) == 10.0

    def test_excludes_bool_runtime_value(self):
        # bool subclasses int; a truthy flag keyed *_runtime must not count as 1s.
        m = {"weird_runtime": True, "train_runtime": 10.0}
        assert _sum_gpu_runtime_seconds(m) == 10.0

    def test_empty(self):
        assert _sum_gpu_runtime_seconds({}) == 0.0


class TestExtractTrainingRuntimeSeconds:
    def test_rounds_total_including_nested(self):
        m = {"train_runtime": 12.6, "dpo": {"dpo_runtime": 2.4}}
        assert _extract_training_runtime_seconds(m) == 15

    def test_aligned_run_is_not_undercharged(self):
        # The regression guard: SFT + DPO both counted (was SFT-only before).
        sft_only = _sum_gpu_runtime_seconds({"sft_train_runtime": 200.0})
        with_dpo = _sum_gpu_runtime_seconds(
            {"sft_train_runtime": 200.0, "dpo": {"dpo_runtime": 120.0}}
        )
        assert with_dpo > sft_only
        assert with_dpo == 320.0
