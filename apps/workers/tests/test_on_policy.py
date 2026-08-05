"""On-policy configuration, validated before anything is paid for.

Every rule here mirrors one TRL enforces inside the trainer — i.e. after a GPU is
running and a 30B teacher has finished loading its weights. Catching them from a
plain dict is the difference between a validation error and a wasted container.
"""

import re
from pathlib import Path

import pytest

from src.activities.on_policy import (
    DEFAULT_BETA,
    DEFAULT_EPOCHS,
    DEFAULT_LEARNING_RATE,
    DEFAULT_LMBDA,
    SERVER_REVERSE_KL_TOP_K,
    OnPolicyConfigError,
    plan_on_policy,
    resolve_generation_batch_size,
    trainer_config_kwargs,
)

TEACHER = {"teacher_model": "Qwen/Qwen3-32B", "teacher_url": "http://127.0.0.1:8000"}


def plan(hp=None, **overrides):
    return plan_on_policy({**(hp or {})}, **{**TEACHER, **overrides})


def test_defaults_are_reverse_kl_fully_on_policy():
    resolved = plan()

    assert resolved.beta == DEFAULT_BETA == 1.0
    assert resolved.lmbda == DEFAULT_LMBDA == 1.0
    assert resolved.objective == "jsd"
    assert resolved.temperature == 1.0


def test_top_k_is_one_because_the_endpoint_cannot_answer_more():
    """A server teacher exposes only its own top-k ranking, so reverse KL has
    exactly the sampled token plus a tail bucket to work with."""
    resolved = plan()

    assert resolved.loss_top_k == SERVER_REVERSE_KL_TOP_K == 1
    assert resolved.loss_add_tail is True


def test_forward_kl_is_refused_as_a_repeat_of_the_offline_stages():
    with pytest.raises(OnPolicyConfigError, match="already do"):
        plan({"distill_beta": 0.0})


def test_beta_outside_the_unit_interval_is_refused():
    with pytest.raises(OnPolicyConfigError, match="between 0.0 and 1.0"):
        plan({"distill_beta": 1.5})


def test_lambda_can_mix_in_off_policy_batches():
    assert plan({"on_policy_lambda": 0.5}).lmbda == 0.5


def test_unknown_objective_names_what_is_available():
    with pytest.raises(OnPolicyConfigError, match="jsd, iw_opd"):
        plan({"distill_objective": "kl_but_better"})


def test_the_importance_weighted_objective_requires_full_on_policy_data():
    """TRL rejects this combination internally; refusing it here keeps the message
    in terms of the knobs this platform actually exposes."""
    with pytest.raises(OnPolicyConfigError, match="on_policy_lambda = 1.0"):
        plan({"distill_objective": "iw_opd", "on_policy_lambda": 0.8})


def test_the_importance_weighted_objective_is_reachable_by_configuration():
    assert plan({"distill_objective": "iw_opd"}).objective == "iw_opd"


def test_zero_temperature_is_refused():
    with pytest.raises(OnPolicyConfigError, match="must be positive"):
        plan({"rollout_temperature": 0.0})


def test_a_missing_teacher_url_is_a_configuration_error():
    with pytest.raises(OnPolicyConfigError, match="teacher server url"):
        plan(teacher_url="")


def test_a_missing_teacher_model_is_a_configuration_error():
    with pytest.raises(OnPolicyConfigError, match="teacher model"):
        plan(teacher_model="")


def test_generation_batch_size_is_derived_the_way_trl_derives_it():
    assert (
        resolve_generation_batch_size(
            per_device_train_batch_size=2,
            gradient_accumulation_steps=8,
            num_generations=4,
        )
        == 4
    )


def test_a_num_generations_that_does_not_divide_the_batch_is_refused():
    with pytest.raises(OnPolicyConfigError, match="must divide"):
        resolve_generation_batch_size(
            per_device_train_batch_size=1,
            gradient_accumulation_steps=5,
            num_generations=2,
        )


def test_an_explicit_generation_batch_size_must_satisfy_trls_identity():
    with pytest.raises(OnPolicyConfigError, match="must equal"):
        resolve_generation_batch_size(
            per_device_train_batch_size=1,
            gradient_accumulation_steps=4,
            num_generations=1,
            requested=3,
        )


def test_a_consistent_explicit_generation_batch_size_is_kept():
    assert (
        resolve_generation_batch_size(
            per_device_train_batch_size=1,
            gradient_accumulation_steps=4,
            num_generations=1,
            requested=4,
        )
        == 4
    )


def test_non_numeric_hyperparams_are_refused_with_the_field_named():
    with pytest.raises(OnPolicyConfigError, match="gradient_accumulation_steps"):
        plan({"gradient_accumulation_steps": "lots"})


def test_config_kwargs_put_the_teacher_behind_the_server_flag():
    kwargs = trainer_config_kwargs(plan(), output_dir="/tmp/out", hp={})

    assert kwargs["use_teacher_server"] is True
    assert kwargs["teacher_model_server_url"] == "http://127.0.0.1:8000"
    assert kwargs["teacher_model_name_or_path"] == "Qwen/Qwen3-32B"
    assert kwargs["loss_top_k"] == 1
    assert kwargs["beta"] == 1.0


def test_config_kwargs_pin_the_teacher_revision_when_provenance_has_one():
    kwargs = trainer_config_kwargs(plan(teacher_revision="9216db57"), output_dir="/tmp/out", hp={})

    assert kwargs["teacher_model_revision"] == "9216db57"


def test_config_kwargs_omit_the_revision_when_unpinned():
    assert "teacher_model_revision" not in trainer_config_kwargs(
        plan(), output_dir="/tmp/out", hp={}
    )


def test_the_trainer_reads_the_epoch_count_the_rest_of_the_platform_writes():
    """`epochs` is nobody's key. Reading it meant the API priced three epochs of
    rollouts and the trainer ran one, and setting num_train_epochs did nothing."""
    kwargs = trainer_config_kwargs(plan(), output_dir="/tmp/out", hp={"num_train_epochs": 2})

    assert kwargs["num_train_epochs"] == 2.0


def test_the_epoch_default_matches_what_the_api_quotes():
    """Both sides default to the same number or the estimate is wrong by an epoch of
    generated tokens. The Rust constant is asserted against this in on_policy.rs."""
    assert DEFAULT_EPOCHS == 3
    assert trainer_config_kwargs(plan(), output_dir="/tmp/out", hp={})["num_train_epochs"] == 3.0


def test_rollouts_use_hf_generate_by_default():
    """Colocated vLLM would compete with the student for memory, and an
    out-of-memory kill costs the whole container."""
    assert trainer_config_kwargs(plan(), output_dir="/tmp/out", hp={})["use_vllm"] is False


def test_vllm_rollouts_are_available_by_configuration():
    kwargs = trainer_config_kwargs(plan({"use_vllm_rollouts": True}), output_dir="/tmp/out", hp={})

    assert kwargs["use_vllm"] is True


def test_the_importance_weighted_objective_forces_weight_sync_when_vllm_generates():
    """TRL requires rollouts to come from the current weights for this objective."""
    kwargs = trainer_config_kwargs(
        plan({"distill_objective": "iw_opd", "use_vllm_rollouts": True}),
        output_dir="/tmp/out",
        hp={},
    )

    assert kwargs["vllm_sync_frequency"] == 1


def test_no_sync_frequency_is_forced_for_the_default_objective():
    kwargs = trainer_config_kwargs(plan({"use_vllm_rollouts": True}), output_dir="/tmp/out", hp={})

    assert "vllm_sync_frequency" not in kwargs


def test_batch_identity_holds_in_the_config_it_hands_trl():
    """The invariant, asserted on the actual kwargs rather than the plan: TRL
    recomputes it and raises inside the trainer if it does not hold."""
    kwargs = trainer_config_kwargs(
        plan({"per_device_train_batch_size": 2, "gradient_accumulation_steps": 8}),
        output_dir="/tmp/out",
        hp={},
    )

    assert (
        kwargs["generation_batch_size"] * kwargs["num_generations"]
        == kwargs["per_device_train_batch_size"] * kwargs["gradient_accumulation_steps"]
    )


def test_the_learning_rate_default_matches_what_the_api_injects():
    """The API always writes a learning_rate, so this default is only reachable if
    the two ever disagree — and 2e-4 on an already-trained adapter overwrites it.
    Read from the Rust source rather than restated, so a change there fails here.
    """
    service = (
        Path(__file__).resolve().parents[3] / "crates/api/src/services/training_job_service.rs"
    ).read_text(encoding="utf-8")
    declared = re.search(r"ON_POLICY_LEARNING_RATE: f64 = ([\d.e-]+);", service)
    assert declared, "ON_POLICY_LEARNING_RATE is not declared in the training job service"

    assert float(declared.group(1)) == DEFAULT_LEARNING_RATE


def test_an_improve_pass_is_not_priced_at_the_from_scratch_rate():
    service = (
        Path(__file__).resolve().parents[3] / "crates/api/src/services/training_job_service.rs"
    ).read_text(encoding="utf-8")
    from_scratch = re.search(r"DEFAULT_LEARNING_RATE: f64 = ([\d.e-]+);", service)
    assert from_scratch, "DEFAULT_LEARNING_RATE is not declared in the training job service"

    assert float(from_scratch.group(1)) != DEFAULT_LEARNING_RATE
