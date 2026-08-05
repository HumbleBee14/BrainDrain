"""How an admitted on-policy plan reaches the training activity.

The seam matters because the two fidelity methods diverge here: `logit` runs a
scoring pass first, `on_policy` must not, since the text its teacher grades does
not exist until the student writes it. And both share the rule that a caller
cannot name the teacher we boot on our own GPU.
"""

import pytest

from src.workflows.train import (
    DISTILL_METHOD_HYPERPARAM,
    EPOCHS_HYPERPARAM,
    NO_PARENT_IN_PLAN_MESSAGE,
    ON_POLICY_METHOD,
    PARENT_ADAPTER_HYPERPARAM,
    TEACHER_MODEL_HYPERPARAM,
    TEACHER_PRECISION_HYPERPARAM,
    TEACHER_REVISION_HYPERPARAM,
    borrowed_fidelity_keys,
    extraction_plan,
    hyperparams_with_live_teacher,
    unsupported_plan_reason,
)

PLAN = {
    "distill_method": "on_policy",
    "teacher_model": "Qwen/Qwen3-32B",
    "teacher_revision": "9216db57",
    "precision": "bf16",
    "gpu_class": "a10080gb_dual",
    "epochs": 3,
    "parent_adapter_path": "tenants/t/models/parent/",
}


def test_an_on_policy_plan_is_carried_on_the_same_teacher_config_block():
    assert extraction_plan({"extraction": PLAN}) == PLAN


def test_a_job_with_no_teacher_config_is_untouched():
    """Every mode that predates fidelity upgrades must reach training unchanged."""
    assert extraction_plan(None) is None
    assert extraction_plan({}) is None


def test_an_on_policy_plan_is_supported():
    assert unsupported_plan_reason(PLAN, "distill") is None


def test_a_fidelity_plan_outside_distill_mode_is_refused():
    assert "no meaning for training mode" in unsupported_plan_reason(PLAN, "quick")


def test_an_unknown_method_is_refused_before_a_gpu_is_booked():
    plan = {**PLAN, "distill_method": "telepathy"}

    assert "Unsupported distillation method" in unsupported_plan_reason(plan, "distill")


def test_a_plan_with_no_teacher_is_refused():
    plan = {**PLAN, "teacher_model": ""}

    assert unsupported_plan_reason(plan, "distill") == "The fidelity plan names no teacher model"


def test_the_teacher_reaches_training_pinned_to_its_revision():
    resolved = hyperparams_with_live_teacher({}, PLAN)

    assert resolved[DISTILL_METHOD_HYPERPARAM] == ON_POLICY_METHOD
    assert resolved[TEACHER_MODEL_HYPERPARAM] == "Qwen/Qwen3-32B"
    assert resolved[TEACHER_REVISION_HYPERPARAM] == "9216db57"
    assert resolved[TEACHER_PRECISION_HYPERPARAM] == "bf16"


def test_the_run_trains_the_epochs_the_quote_was_priced_from():
    """Rollouts are generated per epoch, so an epoch count the API did not price is
    teacher time nobody was charged for. The plan's number wins over a stale one
    sitting in hyperparams."""
    resolved = hyperparams_with_live_teacher({EPOCHS_HYPERPARAM: 9}, PLAN)

    assert resolved[EPOCHS_HYPERPARAM] == 3


def test_a_plan_admitted_before_parents_were_recorded_is_refused():
    """A job approved from `cost_approval` replays its persisted plan verbatim, so
    one admitted by an older binary can still arrive here. Refusing it says so;
    defaulting would train from the base model and grade an untrained student."""
    legacy = {k: v for k, v in PLAN.items() if k != PARENT_ADAPTER_HYPERPARAM}

    assert unsupported_plan_reason(legacy, "distill") == NO_PARENT_IN_PLAN_MESSAGE


def test_the_logit_path_needs_no_parent_to_continue_from():
    """Only an improve pass continues something. Stage 2 trains from scratch on
    stored distributions, and must not be refused for a key it never had."""
    plan = {k: v for k, v in PLAN.items() if k != PARENT_ADAPTER_HYPERPARAM}
    plan["distill_method"] = "logit"

    assert unsupported_plan_reason(plan, "distill") is None


def test_the_parent_adapter_reaches_training_as_a_platform_owned_key():
    """It names storage the worker reads, so a caller must not be able to supply
    one — the same rule the teacher model and artifact prefix are under."""
    resolved = hyperparams_with_live_teacher({}, PLAN)

    assert resolved[PARENT_ADAPTER_HYPERPARAM] == "tenants/t/models/parent/"
    assert PARENT_ADAPTER_HYPERPARAM in borrowed_fidelity_keys(resolved)


def test_a_caller_cannot_choose_the_adapter_the_run_continues_from():
    resolved = hyperparams_with_live_teacher(
        {PARENT_ADAPTER_HYPERPARAM: "tenants/other/models/theirs/"}, PLAN
    )

    assert resolved[PARENT_ADAPTER_HYPERPARAM] == "tenants/t/models/parent/"


def test_a_plan_without_an_epoch_count_leaves_hyperparams_alone():
    """Jobs admitted before the plan carried epochs still train what they were
    given, rather than silently switching to a default."""
    resolved = hyperparams_with_live_teacher(
        {EPOCHS_HYPERPARAM: 2}, {k: v for k, v in PLAN.items() if k != "epochs"}
    )

    assert resolved[EPOCHS_HYPERPARAM] == 2


def test_an_unpinned_teacher_carries_no_revision_key():
    resolved = hyperparams_with_live_teacher({}, {**PLAN, "teacher_revision": ""})

    assert TEACHER_REVISION_HYPERPARAM not in resolved


def test_caller_supplied_knobs_survive_the_merge():
    """The advanced knobs are the user's; only teacher identity is the platform's."""
    resolved = hyperparams_with_live_teacher({"on_policy_lambda": 0.5}, PLAN)

    assert resolved["on_policy_lambda"] == 0.5


@pytest.mark.parametrize(
    "key",
    [
        DISTILL_METHOD_HYPERPARAM,
        TEACHER_MODEL_HYPERPARAM,
        TEACHER_REVISION_HYPERPARAM,
        TEACHER_PRECISION_HYPERPARAM,
        "teacher_artifacts_prefix",
    ],
)
def test_a_caller_cannot_supply_a_platform_owned_key(key):
    """Naming the teacher would mean booting a model of the caller's choosing on our
    metered GPU; naming the artifact prefix would mean reading another tenant's
    distributions. Both are rejected before the run starts."""
    assert borrowed_fidelity_keys({key: "anything"}) == [key]


def test_ordinary_hyperparams_are_not_mistaken_for_platform_keys():
    assert borrowed_fidelity_keys({EPOCHS_HYPERPARAM: 3, "learning_rate": 1e-5}) == []


def test_every_key_the_platform_writes_is_also_a_key_it_refuses_to_accept():
    """The invariant behind both functions: anything written from an admitted plan
    must be rejected when it arrives from a caller, or the guard has a hole.

    Epochs are exempt, and only epochs: the count is priced rather than privileged,
    so a caller may name it and the quote is computed from what they named. Nothing
    else the plan writes is safe to accept from outside.
    """
    written = set(hyperparams_with_live_teacher({}, PLAN)) - {EPOCHS_HYPERPARAM}

    assert written <= set(borrowed_fidelity_keys(dict.fromkeys(written, "x")))
