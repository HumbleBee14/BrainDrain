def test_build_s3_client_returns_bucket(monkeypatch):
    for k, v in {
        "APP_DATABASE_URL": "postgresql://placeholder:x@127.0.0.1/none",
        "APP_S3_ACCESS_KEY": "ak",
        "APP_S3_SECRET_KEY": "sk",
        "APP_S3_BUCKET": "mybucket",
        "APP_S3_ENDPOINT": "http://s3.example.com",
    }.items():
        monkeypatch.setenv(k, v)

    from src.modal_runtime import build_s3_client, build_settings

    settings = build_settings()
    client, bucket = build_s3_client(settings)
    assert bucket == "mybucket"
    assert client is not None
