"""AES-256-GCM handling for tenant secrets stored in the database.

Stored format (shared with the Rust API): "enc:v1:" + base64(nonce || ciphertext)
with a random 12-byte nonce. Values without the prefix are legacy plaintext and
pass through unchanged. The key is base64-encoded 32 bytes
(APP_SETTINGS_ENCRYPTION_KEY, same value as the API's SETTINGS_ENCRYPTION_KEY).
"""

import base64
import binascii
import os

from cryptography.exceptions import InvalidTag
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

ENC_PREFIX = "enc:v1:"
_NONCE_LEN = 12


class SecretCipherError(Exception):
    """An encrypted tenant secret cannot be handled (missing/wrong key, tampering)."""


def _load_key(key_b64: str) -> bytes:
    try:
        key = base64.b64decode(key_b64, validate=True)
    except (ValueError, binascii.Error) as e:
        raise SecretCipherError("settings encryption key must be valid base64") from e
    if len(key) != 32:
        raise SecretCipherError("settings encryption key must decode to exactly 32 bytes")
    return key


def encrypt_secret(plaintext: str, key_b64: str) -> str:
    """Encrypt a secret into the enc:v1 stored format."""
    key = _load_key(key_b64)
    nonce = os.urandom(_NONCE_LEN)
    ciphertext = AESGCM(key).encrypt(nonce, plaintext.encode("utf-8"), None)
    return ENC_PREFIX + base64.b64encode(nonce + ciphertext).decode("ascii")


def decrypt_secret(value: str, key_b64: str | None) -> str:
    """Decrypt a stored secret; legacy plaintext passes through unchanged.

    Raises SecretCipherError for an encrypted value with a missing/wrong key or
    a tampered payload — callers must fail loud, never use the raw enc:v1 blob.
    """
    if not value.startswith(ENC_PREFIX):
        return value
    if not key_b64:
        raise SecretCipherError(
            "encountered an encrypted tenant secret but APP_SETTINGS_ENCRYPTION_KEY is not set"
        )
    key = _load_key(key_b64)
    try:
        payload = base64.b64decode(value[len(ENC_PREFIX) :], validate=True)
    except (ValueError, binascii.Error) as e:
        raise SecretCipherError("encrypted tenant secret is malformed") from e
    if len(payload) <= _NONCE_LEN:
        raise SecretCipherError("encrypted tenant secret is malformed")
    try:
        plaintext = AESGCM(key).decrypt(payload[:_NONCE_LEN], payload[_NONCE_LEN:], None)
    except InvalidTag as e:
        raise SecretCipherError(
            "tenant secret failed to decrypt (wrong key or tampered value)"
        ) from e
    return plaintext.decode("utf-8")
