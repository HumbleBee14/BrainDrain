variable "name" {
  description = "Prefix for resource names."
  default     = "ekcron"
}

variable "tenancy_ocid" {
  description = "Root tenancy OCID (from ~/.oci/config)."
  type        = string
}

variable "compartment_ocid" {
  description = "Compartment OCID to deploy into."
  type        = string
}

variable "instance_shape" {
  default = "VM.Standard.A1.Flex"
}
variable "ocpus" {
  default = 2
}
variable "memory_gb" {
  default = 12
}
variable "boot_volume_gb" {
  description = "Boot disk size in GB (minimum 50)."
  default     = 50
}

variable "ssh_public_key_path" {
  default = "~/.ssh/ekcron_box.pub"
}

# TCP ports open to the internet. Tighten port 22 to your IP for real hardening.
variable "ingress_tcp_ports" {
  type = list(object({ port = number, cidr = string }))
  default = [
    { port = 22, cidr = "0.0.0.0/0" },
    { port = 80, cidr = "0.0.0.0/0" },
    { port = 443, cidr = "0.0.0.0/0" },
  ]
}

# --- Cloudflare DNS (api.ekcron.com → box) ---
variable "cloudflare_api_token" {
  description = "Cloudflare API token with Zone:DNS:Edit for ekcron.com."
  type        = string
  sensitive   = true
}
variable "cloudflare_zone_id" {
  description = "Zone ID for ekcron.com."
  type        = string
  default     = "10169f033b8e0f74f7ae3941e72b3962"
}
variable "api_domain" {
  default = "api.ekcron.com"
}
