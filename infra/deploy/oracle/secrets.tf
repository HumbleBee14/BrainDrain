# App secrets: one Vault secret holds the box's entire .env as a blob.
# Terraform manages the container (vault/key/secret + who may read it);
# the VALUE is set out-of-band via set-env-secret.sh so it never enters
# git or Terraform state. The box reads it with instance-principal auth
# (render-env.sh) — the OCI equivalent of an EC2 role + Secrets Manager.

resource "oci_kms_vault" "vault" {
  compartment_id = var.compartment_ocid
  display_name   = "${var.name}-vault"
  vault_type     = "DEFAULT"
}

resource "oci_kms_key" "env_key" {
  compartment_id      = var.compartment_ocid
  display_name        = "${var.name}-env-key"
  management_endpoint = oci_kms_vault.vault.management_endpoint
  protection_mode     = "SOFTWARE"
  key_shape {
    algorithm = "AES"
    length    = 32
  }
}

resource "oci_vault_secret" "box_env" {
  compartment_id = var.compartment_ocid
  vault_id       = oci_kms_vault.vault.id
  key_id         = oci_kms_key.env_key.id
  secret_name    = "${var.name}-box-env"

  # Placeholder only. Real content is pushed with set-env-secret.sh and
  # ignored here so plans never diff (or store) the actual credentials.
  secret_content {
    content_type = "BASE64"
    content      = base64encode("# managed via set-env-secret.sh\n")
  }

  lifecycle {
    ignore_changes = [secret_content]
  }
}

resource "oci_identity_dynamic_group" "box" {
  compartment_id = var.tenancy_ocid
  name           = "${var.name}-box-dg"
  description    = "The ${var.name} deploy box, identified by instance OCID."
  matching_rule  = "ANY {instance.id = '${oci_core_instance.box.id}'}"
}

# Read-only on purpose: the box can fetch its config but never modify it.
# Writes stay with the operator (laptop ~/.oci/config) via set-env-secret.sh.
resource "oci_identity_policy" "box_read_env" {
  compartment_id = var.compartment_ocid
  name           = "${var.name}-box-read-env"
  description    = "Allow the ${var.name} box to read the app env secret."
  statements = [
    "Allow dynamic-group ${oci_identity_dynamic_group.box.name} to read secret-family in compartment id ${var.compartment_ocid} where target.secret.id = '${oci_vault_secret.box_env.id}'",
  ]
}
