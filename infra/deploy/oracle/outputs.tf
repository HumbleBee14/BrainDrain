output "public_ip" {
  description = "Reserved public IP — point api.ekcron.com here in Cloudflare DNS."
  value       = oci_core_public_ip.reserved.ip_address
}

output "ssh" {
  description = "SSH into the box."
  value       = "ssh -i ~/.ssh/ekcron_box ubuntu@${oci_core_public_ip.reserved.ip_address}"
}
