"""Activity heartbeats that tolerate running outside Temporal.

Core compute functions are shared by the local provider (inside an activity) and
the cloud provider (inside a Modal container, where no activity context exists
and `activity.heartbeat` raises RuntimeError).
"""

import logging

from temporalio import activity

logger = logging.getLogger("platform.heartbeat")


def safe_heartbeat(*details: object) -> None:
    """Report progress when in an activity; no-op when not."""
    try:
        activity.heartbeat(*details)
    except RuntimeError:
        pass
    except Exception as exc:
        logger.debug("Heartbeat failed: %s", exc)
