"""Fetch-time SSRF re-validation for tenant-supplied URLs.

The API validates URLs at save time, but DNS can be re-pointed between save
and fetch (rebinding TOCTOU) — so the worker re-validates just before use.
Residual gap: DNS may still change between this resolution and the actual
connect; this module only closes the save-to-fetch window.
"""

import ipaddress
import socket
from urllib.parse import urlsplit


class UnsafeUrlError(ValueError):
    """Raised when a URL points at a non-public or malformed destination."""


def url_guard_active(settings) -> bool:
    """Whether fetch-time URL validation is enforced.

    APP_URL_GUARD_ENABLED overrides; otherwise enforced only in production so
    dev keeps working against localhost LLM endpoints.
    """
    if settings.url_guard_enabled is not None:
        return settings.url_guard_enabled
    return settings.environment == "production"


def _is_forbidden(ip: ipaddress.IPv4Address | ipaddress.IPv6Address) -> bool:
    mapped = getattr(ip, "ipv4_mapped", None)
    if mapped is not None:
        ip = mapped
    return (
        ip.is_private  # RFC1918 v4, unique-local v6, and friends
        or ip.is_loopback
        or ip.is_link_local  # includes cloud metadata 169.254.169.254
        or ip.is_multicast
        or ip.is_reserved
        or ip.is_unspecified
        or bool(getattr(ip, "is_site_local", False))
    )


def assert_safe_public_url(url: str) -> None:
    """Raise UnsafeUrlError unless `url` is plain http(s) to a public address.

    Resolves ALL A/AAAA records and rejects if ANY points at a private,
    loopback, link-local, metadata, multicast, reserved, or unspecified
    address. Blocking DNS call — run via asyncio.to_thread from async code.
    """
    parts = urlsplit(url)

    if parts.scheme not in ("http", "https"):
        raise UnsafeUrlError(f"URL scheme must be http(s), got {parts.scheme!r}")
    if parts.username is not None or parts.password is not None:
        raise UnsafeUrlError("URL must not contain userinfo")

    host = parts.hostname
    if not host:
        raise UnsafeUrlError("URL has no host")

    try:
        literal = ipaddress.ip_address(host)
    except ValueError:
        literal = None

    if literal is not None:
        if _is_forbidden(literal):
            raise UnsafeUrlError(f"URL points at non-public address {literal}")
        return

    port = parts.port or (443 if parts.scheme == "https" else 80)
    try:
        infos = socket.getaddrinfo(host, port, proto=socket.IPPROTO_TCP)
    except socket.gaierror as e:
        raise UnsafeUrlError(f"cannot resolve host {host!r}: {e}") from e

    if not infos:
        raise UnsafeUrlError(f"host {host!r} resolved to no addresses")

    for info in infos:
        resolved = ipaddress.ip_address(info[4][0])
        if _is_forbidden(resolved):
            raise UnsafeUrlError(f"host {host!r} resolves to non-public address {resolved}")
