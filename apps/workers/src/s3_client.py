"""S3-compatible object-storage client factory.

ONE boto3 client-construction path that works across AWS S3, MinIO, and
Cloudflare R2 (and other S3-compatible stores), selectable purely by the
`APP_S3_*` config — no code change to switch backends. This is the pluggable
storage seam: users who don't want AWS can point `APP_S3_ENDPOINT` at R2,
MinIO, Backblaze B2, etc.

The botocore `Config` below is the compatibility layer, and every setting is
safe for AWS/MinIO/R2 alike (verified against current Cloudflare R2 + botocore
1.42 behavior):

- `request_checksum_calculation` / `response_checksum_validation` = "when_required":
  botocore >= 1.36 defaults these to "when_supported", which adds a client-side
  CRC32 checksum to every PutObject. Non-AWS stores (R2, and some MinIO
  versions) reject that on a plain single-object upload (HTTP 501 / "Not
  Implemented" / checksum-mismatch). "when_required" restores the pre-1.36
  behavior, which real AWS S3 still fully supports — so it degrades nothing.
- path-style addressing + SigV4: the universally-compatible pair. Virtual-hosted
  addressing needs wildcard-subdomain support that not every store offers;
  path-style works everywhere (AWS still supports it, MinIO/R2 recommend it).

Example endpoints:
  AWS S3   : leave APP_S3_ENDPOINT at the regional endpoint; APP_S3_REGION=<region>
  MinIO    : http://minio:9000                      ; APP_S3_REGION=us-east-1
  R2       : https://<ACCOUNT_ID>.r2.cloudflarestorage.com ; APP_S3_REGION=auto
"""

from botocore.config import Config

from src.config import WorkerSettings

# Shared, backend-agnostic client config. Safe for AWS S3, MinIO, and R2.
S3_COMPAT_CONFIG = Config(
    signature_version="s3v4",
    s3={"addressing_style": "path"},
    request_checksum_calculation="when_required",
    response_checksum_validation="when_required",
)


def create_s3_client(settings: WorkerSettings):
    """Build a boto3 S3 client that works across AWS/MinIO/R2 from `APP_S3_*` config."""
    import boto3

    return boto3.client(
        "s3",
        endpoint_url=settings.s3_endpoint,
        aws_access_key_id=settings.s3_access_key,
        aws_secret_access_key=settings.s3_secret_key,
        region_name=settings.s3_region,
        config=S3_COMPAT_CONFIG,
    )
