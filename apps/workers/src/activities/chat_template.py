"""Single source of truth for chat formatting across training and evaluation.

Fine-tuned models are served through vLLM/SGLang/TGI, which format incoming
chat messages with the model's OWN tokenizer chat template. To avoid
train/serve skew, training and evaluation MUST format with that same template
rather than a hardcoded string.

Only Qwen is natively ChatML (`<|im_start|>` / `<|im_end|>`). Llama, Mistral,
Phi and Gemma each use a different template, and Gemma rejects a `system` role
outright. A hardcoded ChatML formatter therefore silently degraded four of the
catalog's five non-Qwen base models: wrong role markers, no EOS learned, and a
dropped/rejected system prompt — while the eval harness applied the same wrong
format, so it could never surface the regression.

Every formatter now routes through `render_chat`, which delegates to the
tokenizer's `apply_chat_template`. `ensure_chat_template` guarantees a template
exists (installing a ChatML fallback for base models that ship none) and, since
the tokenizer is saved alongside the adapter, that fallback is applied
identically at serve time — consistent by construction.
"""

import logging

logger = logging.getLogger("platform.training.chat_template")

# Standard ChatML template, used ONLY as a fallback for base models whose
# tokenizer ships no chat_template. Assigning it to the tokenizer means the
# exact same template is persisted with the adapter and applied at serve time.
CHATML_FALLBACK_TEMPLATE = (
    "{% for message in messages %}"
    "{{'<|im_start|>' + message['role'] + '\n' + message['content'] + '<|im_end|>' + '\n'}}"
    "{% endfor %}"
    "{% if add_generation_prompt %}{{'<|im_start|>assistant\n'}}{% endif %}"
)


def ensure_chat_template(tokenizer):
    """Guarantee `tokenizer` can render chat messages, and return it.

    Instruct/chat models ship a `chat_template`; plain base models may not.
    When one is absent we install a standard ChatML fallback and warn loudly —
    never silently. Because the tokenizer is saved with the trained adapter (and
    thus loaded by the serving backend), the fallback is applied identically at
    train and serve time, so no skew is introduced.
    """
    if getattr(tokenizer, "chat_template", None):
        return tokenizer
    logger.warning(
        "Base model tokenizer has no chat_template; installing a ChatML fallback. "
        "It is saved with the adapter so serving uses the same template, but an "
        "instruct/chat base model is strongly preferred for chat fine-tuning."
    )
    tokenizer.chat_template = CHATML_FALLBACK_TEMPLATE
    return tokenizer


def render_chat(tokenizer, messages, *, add_generation_prompt=False):
    """Render a message list to text using the model's own chat template.

    This is the exact formatting the serving backend applies, so text produced
    here for training/eval matches what the deployed model sees at inference.

    Set `add_generation_prompt=True` to append the leading assistant marker for
    generation prompts (evaluation, on-policy sampling); leave it False for full
    supervised examples that already include the assistant turn.
    """
    return tokenizer.apply_chat_template(
        messages,
        tokenize=False,
        add_generation_prompt=add_generation_prompt,
    )


def split_prompt_and_response(messages):
    """Split a conversation into ``(prompt_messages, gold_response)``.

    ``prompt_messages`` is every turn up to (but excluding) the final assistant
    turn; ``gold_response`` is that assistant turn's text content. Returns
    ``(None, None)`` when there is no usable trailing assistant turn to learn
    from (empty conversation, no assistant reply, or an empty gold answer).
    """
    if not messages:
        return None, None

    last_assistant = None
    for i in range(len(messages) - 1, -1, -1):
        if messages[i].get("role") == "assistant":
            last_assistant = i
            break

    # Need at least one preceding (prompt) turn and a non-empty gold answer.
    if last_assistant is None or last_assistant == 0:
        return None, None

    prompt_messages = messages[:last_assistant]
    gold = (messages[last_assistant].get("content") or "").strip()
    if not gold:
        return None, None

    return prompt_messages, gold
