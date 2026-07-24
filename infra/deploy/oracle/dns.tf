provider "cloudflare" {
  api_token = var.cloudflare_api_token
}

# api.ekcron.com → the box's reserved IP. Because `content` references the OCI
# output, rebuilding the box updates DNS automatically on the next `apply`.
resource "cloudflare_dns_record" "api" {
  zone_id = var.cloudflare_zone_id
  name    = var.api_domain
  type    = "A"
  content = oci_core_public_ip.reserved.ip_address
  ttl     = 1
  # DNS-only: the box terminates TLS with Caddy (Let's Encrypt). Re-enable the
  # proxy later with a Cloudflare origin certificate to regain WAF/DDoS + hidden origin.
  proxied = false
}
