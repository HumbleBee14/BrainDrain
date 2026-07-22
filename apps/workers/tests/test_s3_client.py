"""The shared S3 client factory must carry the backend-agnostic compat config
(AWS / MinIO / Cloudflare R2) and build a client from APP_S3_* settings."""

from src.s3_client import S3_COMPAT_CONFIG, create_s3_client


def test_compat_config_disables_default_checksums():
    # botocore >=1.36 defaults to "when_supported", which breaks R2/MinIO uploads.
    assert S3_COMPAT_CONFIG.request_checksum_calculation == "when_required"
    assert S3_COMPAT_CONFIG.response_checksum_validation == "when_required"


def test_compat_config_uses_path_style_and_sigv4():
    assert S3_COMPAT_CONFIG.signature_version == "s3v4"
    assert S3_COMPAT_CONFIG.s3["addressing_style"] == "path"


def test_create_s3_client_builds_against_r2_style_endpoint(monkeypatch):
    for k, v in {
        "APP_DATABASE_URL": "postgresql://placeholder:x@127.0.0.1/none",
        "APP_S3_ACCESS_KEY": "ak",
        "APP_S3_SECRET_KEY": "sk",
        "APP_S3_BUCKET": "bkt",
        "APP_S3_ENDPOINT": "https://acct123.r2.cloudflarestorage.com",
        "APP_S3_REGION": "auto",
    }.items():
        monkeypatch.setenv(k, v)

    from src.config import WorkerSettings

    client = create_s3_client(WorkerSettings())
    assert client is not None
    assert client.meta.endpoint_url == "https://acct123.r2.cloudflarestorage.com"
