"""Fetch-time SSRF re-validation (src/url_guard.py).

DNS is always mocked — no live lookups in tests.
"""

import socket

import pytest

from src.tenant_config import get_tenant_llm_config
from src.url_guard import UnsafeUrlError, assert_safe_public_url, url_guard_active


def _addrinfo(*ips):
    return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (ip, 443)) for ip in ips]


def _mock_dns(monkeypatch, *ips):
    monkeypatch.setattr(
        "src.url_guard.socket.getaddrinfo",
        lambda *args, **kwargs: _addrinfo(*ips),
    )


class _Settings:
    def __init__(self, environment="development", url_guard_enabled=None):
        self.environment = environment
        self.url_guard_enabled = url_guard_enabled


# -- literal IP hosts --


@pytest.mark.parametrize(
    "url",
    [
        "http://127.0.0.1:9911/v1",
        "http://10.0.0.5/v1",
        "http://172.16.3.4/v1",
        "http://192.168.1.10/v1",
        "http://169.254.169.254/latest/meta-data/",
        "http://0.0.0.0/v1",
        "http://[::1]/v1",
        "http://[fc00::1]/v1",
        "http://[fe80::1]/v1",
        "http://[::ffff:127.0.0.1]/v1",
    ],
)
def test_private_literal_ips_rejected(url):
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url(url)


def test_public_literal_ip_accepted():
    assert_safe_public_url("https://8.8.8.8/v1")


# -- resolved hostnames (mocked DNS) --


def test_hostname_resolving_private_rejected(monkeypatch):
    _mock_dns(monkeypatch, "10.1.2.3")
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url("https://api.example.com/v1")


def test_hostname_resolving_metadata_rejected(monkeypatch):
    _mock_dns(monkeypatch, "169.254.169.254")
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url("https://api.example.com/v1")


def test_hostname_resolving_public_accepted(monkeypatch):
    _mock_dns(monkeypatch, "93.184.216.34")
    assert_safe_public_url("https://api.example.com/v1")


def test_any_private_record_rejects_mixed_resolution(monkeypatch):
    _mock_dns(monkeypatch, "93.184.216.34", "127.0.0.1")
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url("https://api.example.com/v1")


def test_unresolvable_host_rejected(monkeypatch):
    def _boom(*args, **kwargs):
        raise socket.gaierror("no such host")

    monkeypatch.setattr("src.url_guard.socket.getaddrinfo", _boom)
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url("https://nope.example.com/v1")


# -- scheme / shape --


@pytest.mark.parametrize(
    "url",
    [
        "ftp://example.com/v1",
        "file:///etc/passwd",
        "gopher://example.com",
        "https://user:pass@example.com/v1",
        "https://user@example.com/v1",
        "https:///v1",
        "not a url",
    ],
)
def test_bad_scheme_or_userinfo_rejected(url):
    with pytest.raises(UnsafeUrlError):
        assert_safe_public_url(url)


# -- gating --


def test_guard_active_only_in_production_by_default():
    assert url_guard_active(_Settings(environment="production"))
    assert not url_guard_active(_Settings(environment="development"))


def test_guard_explicit_override_wins():
    assert url_guard_active(_Settings(environment="development", url_guard_enabled=True))
    assert not url_guard_active(_Settings(environment="production", url_guard_enabled=False))


# -- integration with get_tenant_llm_config --


class _FakeDb:
    def __init__(self, settings_value):
        self._settings = settings_value

    async def fetchrow(self, query, tenant_id):
        return {"settings": self._settings}


_TENANT_SETTINGS = {"llm": {"api_base_url": "http://127.0.0.1:9911/v1", "api_key": "sk-x"}}


@pytest.mark.asyncio
async def test_tenant_url_rejected_at_fetch_time_in_production():
    with pytest.raises(UnsafeUrlError):
        await get_tenant_llm_config(
            db=_FakeDb(_TENANT_SETTINGS),
            tenant_id="t-1",
            default_api_base_url="https://api.example.com/v1",
            default_api_key="",
            default_model="m",
            settings=_Settings(environment="production"),
        )


@pytest.mark.asyncio
async def test_tenant_localhost_url_allowed_in_dev():
    config = await get_tenant_llm_config(
        db=_FakeDb(_TENANT_SETTINGS),
        tenant_id="t-1",
        default_api_base_url="https://api.example.com/v1",
        default_api_key="",
        default_model="m",
        settings=_Settings(environment="development"),
    )
    assert config.api_base_url == "http://127.0.0.1:9911/v1"


@pytest.mark.asyncio
async def test_default_url_not_guarded_in_production():
    # No tenant-supplied URL: operator env default is trusted config.
    config = await get_tenant_llm_config(
        db=_FakeDb({"llm": {"api_key": "sk-x"}}),
        tenant_id="t-1",
        default_api_base_url="http://127.0.0.1:9911/v1",
        default_api_key="",
        default_model="m",
        settings=_Settings(environment="production"),
    )
    assert config.api_base_url == "http://127.0.0.1:9911/v1"
