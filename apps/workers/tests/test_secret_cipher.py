"""Tests for AES-256-GCM tenant-secret handling (format shared with the Rust API)."""

import base64

import pytest

from src.secret_cipher import ENC_PREFIX, SecretCipherError, decrypt_secret, encrypt_secret
from src.tenant_config import get_tenant_llm_config

# Base64 of bytes 0x00..0x1f. Test-only key, never used in deployments.
TEST_KEY_B64 = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="

# Must stay in sync with crates/api/src/services/secret_cipher.rs
# (cross_language_test_vector) — proves both implementations share one format.
# key = TEST_KEY_B64, nonce = bytes 0..11, plaintext = "sk-test-secret-key-1234".
CROSS_LANGUAGE_VECTOR = (
    "enc:v1:AAECAwQFBgcICQoLNGn7b6CWtjb+JPT51J1VBuavqgXCSGv5XIfps4HrjZSgvnC5bMoI"
)


class TestDecryptSecret:
    def test_cross_language_vector(self):
        assert decrypt_secret(CROSS_LANGUAGE_VECTOR, TEST_KEY_B64) == "sk-test-secret-key-1234"

    def test_round_trip(self):
        stored = encrypt_secret("sk-test-secret-key-1234", TEST_KEY_B64)
        assert stored.startswith(ENC_PREFIX)
        assert decrypt_secret(stored, TEST_KEY_B64) == "sk-test-secret-key-1234"

    def test_plaintext_passes_through(self):
        assert decrypt_secret("sk-legacy-plain", TEST_KEY_B64) == "sk-legacy-plain"
        assert decrypt_secret("sk-legacy-plain", None) == "sk-legacy-plain"
        assert decrypt_secret("sk-legacy-plain", "") == "sk-legacy-plain"

    def test_encrypted_without_key_raises(self):
        with pytest.raises(SecretCipherError):
            decrypt_secret(CROSS_LANGUAGE_VECTOR, None)
        with pytest.raises(SecretCipherError):
            decrypt_secret(CROSS_LANGUAGE_VECTOR, "")

    def test_wrong_key_raises(self):
        other_key = base64.b64encode(bytes(range(32, 64))).decode()
        with pytest.raises(SecretCipherError):
            decrypt_secret(CROSS_LANGUAGE_VECTOR, other_key)

    def test_tampered_ciphertext_raises(self):
        payload = bytearray(base64.b64decode(CROSS_LANGUAGE_VECTOR[len(ENC_PREFIX) :]))
        payload[-1] ^= 0x01
        tampered = ENC_PREFIX + base64.b64encode(bytes(payload)).decode()
        with pytest.raises(SecretCipherError):
            decrypt_secret(tampered, TEST_KEY_B64)

    def test_malformed_payload_raises(self):
        with pytest.raises(SecretCipherError):
            decrypt_secret(ENC_PREFIX + "not-base64!!", TEST_KEY_B64)
        with pytest.raises(SecretCipherError):
            decrypt_secret(ENC_PREFIX + base64.b64encode(b"short").decode(), TEST_KEY_B64)

    def test_invalid_key_raises(self):
        with pytest.raises(SecretCipherError):
            decrypt_secret(CROSS_LANGUAGE_VECTOR, "not-base64!!")
        with pytest.raises(SecretCipherError):
            decrypt_secret(CROSS_LANGUAGE_VECTOR, base64.b64encode(b"too-short").decode())


class _FakeDb:
    """Stub asyncpg pool returning one tenant settings row."""

    def __init__(self, settings):
        self._settings = settings

    async def fetchrow(self, query, *args):
        return {"settings": self._settings}


def _defaults():
    return {
        "default_api_base_url": "https://default.example/v1",
        "default_api_key": "sk-default",
        "default_model": "default-model",
    }


class TestTenantConfigDecryption:
    @pytest.mark.asyncio
    async def test_encrypted_key_is_decrypted(self):
        db = _FakeDb({"llm": {"api_key": CROSS_LANGUAGE_VECTOR}})
        config = await get_tenant_llm_config(
            db, "tenant-1", **_defaults(), encryption_key=TEST_KEY_B64
        )
        assert config.api_key == "sk-test-secret-key-1234"
        assert config.is_custom is True

    @pytest.mark.asyncio
    async def test_legacy_plaintext_key_passes_through(self):
        db = _FakeDb({"llm": {"api_key": "sk-legacy-plain"}})
        config = await get_tenant_llm_config(
            db, "tenant-1", **_defaults(), encryption_key=TEST_KEY_B64
        )
        assert config.api_key == "sk-legacy-plain"
        assert config.is_custom is True

    @pytest.mark.asyncio
    async def test_encrypted_key_without_encryption_key_raises(self):
        db = _FakeDb({"llm": {"api_key": CROSS_LANGUAGE_VECTOR}})
        with pytest.raises(SecretCipherError):
            await get_tenant_llm_config(db, "tenant-1", **_defaults(), encryption_key=None)

    @pytest.mark.asyncio
    async def test_no_custom_key_falls_back_to_defaults(self):
        db = _FakeDb({"llm": {"model": "custom-model"}})
        config = await get_tenant_llm_config(
            db, "tenant-1", **_defaults(), encryption_key=TEST_KEY_B64
        )
        assert config.api_key == "sk-default"
        assert config.is_custom is False
