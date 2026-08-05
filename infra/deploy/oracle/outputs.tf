output "public_ip" {
  description = "Reserved public IP — point api.ekcron.com here in Cloudflare DNS."
  value       = oci_core_public_ip.reserved.ip_address
}

output "ssh" {
  description = "SSH into the box."
  value       = "ssh -i ~/.ssh/ekcron_box ubuntu@${oci_core_public_ip.reserved.ip_address}"
}

output "env_secret_ocid" {
  description = "Vault secret holding the box .env — consumed by set-env-secret.sh and render-env.sh."
  value       = oci_vault_secret.box_env.id
}
