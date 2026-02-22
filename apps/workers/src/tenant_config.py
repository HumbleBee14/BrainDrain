"""Tenant-specific LLM configuration fetched from DB at activity execution time.

Security: API keys come from the database, never from Temporal workflow payloads.
Fallback: If a tenant has no custom LLM config, uses the worker-level env var defaults.
"""

import json
import logging
from dataclasses import dataclass

logger = logging.getLogger("platform.tenant_config")


@dataclass
class TenantLlmConfig:
    """Resolved LLM configuration for a specific tenant."""

    api_base_url: str
    api_key: str
    model: str
    max_tokens: int
    is_custom: bool  # True if tenant has custom config, False if using defaults


async def get_tenant_llm_config(
    db,
    tenant_id: str,
    default_api_base_url: str,
    default_api_key: str,
    default_model: str,
    default_max_tokens: int = 2000,
) -> TenantLlmConfig:
    """Fetch tenant-specific LLM config from DB, falling back to worker defaults.

    Called at activity execution time (not workflow time) so secrets
    never appear in Temporal workflow history.

    Args:
        db: asyncpg connection pool
        tenant_id: UUID string of the tenant
        default_*: Worker-level env var defaults (used when tenant has no custom config)
    """
    defaults = TenantLlmConfig(
        api_base_url=default_api_base_url,
        api_key=default_api_key,
        model=default_model,
        max_tokens=default_max_tokens,
        is_custom=False,
    )

    try:
        row = await db.fetchrow(
            "SELECT settings FROM tenants WHERE id = $1",
            tenant_id,
        )
    except Exception as e:
        logger.warning("Failed to fetch tenant settings for %s: %s", tenant_id, e)
        return defaults

    if row is None:
        logger.warning("Tenant not found: %s, using defaults", tenant_id)
        return defaults

    settings = row["settings"]
    if isinstance(settings, str):
        settings = json.loads(settings)

    if not isinstance(settings, dict):
        return defaults

    llm = settings.get("llm")
    if not llm or not isinstance(llm, dict):
        return defaults

    # Tenant has custom config — use it, falling back to defaults for missing fields
    api_key = llm.get("api_key") or ""
    has_custom_key = bool(api_key)

    return TenantLlmConfig(
        api_base_url=llm.get("api_base_url") or default_api_base_url,
        api_key=api_key if has_custom_key else default_api_key,
        model=llm.get("model") or default_model,
        max_tokens=llm.get("max_tokens") or default_max_tokens,
        is_custom=has_custom_key,
    )
