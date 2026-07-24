terraform {
  required_version = ">= 1.5"
  required_providers {
    oci = {
      source  = "oracle/oci"
      version = "~> 6.0"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
  }
}

# Auth comes from ~/.oci/config [DEFAULT]. Nothing secret in this file.
provider "oci" {}

# Newer regions (Queretaro) have a single availability domain — fetch, don't hardcode.
data "oci_identity_availability_domains" "ads" {
  compartment_id = var.tenancy_ocid
}

# Latest Canonical Ubuntu 22.04 image for the ARM A1 shape.
data "oci_core_images" "ubuntu" {
  compartment_id           = var.compartment_ocid
  operating_system         = "Canonical Ubuntu"
  operating_system_version = "22.04"
  shape                    = var.instance_shape
  sort_by                  = "TIMECREATED"
  sort_order               = "DESC"
}

# ----------------------------- Network -----------------------------
resource "oci_core_vcn" "vcn" {
  compartment_id = var.compartment_ocid
  cidr_blocks    = ["10.0.0.0/16"]
  display_name   = "${var.name}-vcn"
  dns_label      = "ekcron"
}

resource "oci_core_internet_gateway" "igw" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.vcn.id
  display_name   = "${var.name}-igw"
}

resource "oci_core_route_table" "rt" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.vcn.id
  display_name   = "${var.name}-rt"
  route_rules {
    destination       = "0.0.0.0/0"
    network_entity_id = oci_core_internet_gateway.igw.id
  }
}

resource "oci_core_security_list" "sl" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.vcn.id
  display_name   = "${var.name}-sl"

  egress_security_rules {
    destination = "0.0.0.0/0"
    protocol    = "all"
  }

  # One ingress rule per (port, cidr) in var.ingress_tcp_ports. Tighten SSH to
  # your own IP later by editing that variable.
  dynamic "ingress_security_rules" {
    for_each = var.ingress_tcp_ports
    content {
      protocol = "6" # TCP
      source   = ingress_security_rules.value.cidr
      tcp_options {
        min = ingress_security_rules.value.port
        max = ingress_security_rules.value.port
      }
    }
  }
}

resource "oci_core_subnet" "subnet" {
  compartment_id             = var.compartment_ocid
  vcn_id                     = oci_core_vcn.vcn.id
  cidr_block                 = "10.0.1.0/24"
  display_name               = "${var.name}-subnet"
  route_table_id             = oci_core_route_table.rt.id
  security_list_ids          = [oci_core_security_list.sl.id]
  prohibit_public_ip_on_vnic = false
  dns_label                  = "app"
}

# ----------------------------- Compute -----------------------------
resource "oci_core_instance" "box" {
  compartment_id      = var.compartment_ocid
  availability_domain = data.oci_identity_availability_domains.ads.availability_domains[0].name
  display_name        = "${var.name}-box"
  shape               = var.instance_shape

  shape_config {
    ocpus         = var.ocpus
    memory_in_gbs = var.memory_gb
  }

  create_vnic_details {
    subnet_id        = oci_core_subnet.subnet.id
    assign_public_ip = false # a RESERVED ip is attached below for a stable address
    hostname_label   = "box"
  }

  source_details {
    source_type             = "image"
    source_id               = data.oci_core_images.ubuntu.images[0].id
    boot_volume_size_in_gbs = var.boot_volume_gb
  }

  metadata = {
    ssh_authorized_keys = file(pathexpand(var.ssh_public_key_path))
    user_data           = base64encode(file("${path.module}/cloud-init.yaml"))
  }
}

# ------------------- Reserved (stable) public IP -------------------
# Attach a reserved IP to the instance's primary private IP so the address
# survives instance recreation — DNS (api.ekcron.com) never has to change.
data "oci_core_vnic_attachments" "va" {
  compartment_id = var.compartment_ocid
  instance_id    = oci_core_instance.box.id
}

data "oci_core_private_ips" "pip" {
  vnic_id = data.oci_core_vnic_attachments.va.vnic_attachments[0].vnic_id
}

resource "oci_core_public_ip" "reserved" {
  compartment_id = var.compartment_ocid
  lifetime       = "RESERVED"
  display_name   = "${var.name}-ip"
  private_ip_id  = data.oci_core_private_ips.pip.private_ips[0].id
}
