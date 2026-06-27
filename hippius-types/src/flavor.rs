//! Tenant VM flavor — a strongly-typed bundle of (vCPU, memory, disk)
//! resources mapped from a single human-friendly name.
//!
//! ## Why a Rust enum
//!
//! Today the OrderTicket carries `resource_class: String` (§6 / [`crate::
//! ticket::OrderTicket::resource_class`]) — free-form, used by
//! `agent-tenant-telemetry` as a receipt tag. That works for audit
//! tagging but does NOT bind the per-tenant compute budget to a
//! signed-immutable identifier.
//!
//! This module adds a closed enum (`Small` … `X4large`) whose
//! canonical string form (`"small"` … `"4xlarge"`) is
//! what `vali_create_vm --flavor` writes into the ticket's
//! `resource_class` field. The enum makes the catalogue authoritative:
//!
//! - Adding a new flavor is a code change (vetted, reviewed) — not an
//!   operator string typo.
//! - The per-flavor (vcpus, memory_mb, disk_gb) tuple lives in ONE
//!   place; both vali (mint side) and the future miner-agent
//!   COSE-verifier (enforcement side) read the same numbers.
//! - vCPU count is **measurement-affecting** under SEV-SNP — every
//!   distinct `vcpus` value requires its own §22 allowlist entry. The
//!   closed enum surfaces the size of the operational footprint
//!   (six entries today, not "however many flavors the operator can
//!   type").
//!
//! ## Wire-format scope
//!
//! This PR does NOT change the OrderTicket on-the-wire shape — the
//! ticket continues to carry `resource_class: String`, but
//! `vali_create_vm --flavor` now writes the canonical flavor name
//! into that field. A follow-up PR can bump `SCHEMA_V` and replace
//! `resource_class` with `flavor: Flavor` directly once every consumer
//! (KBS, agent-tenant-telemetry, the L1 minter) has been updated.
//!
//! ## Stable string form
//!
//! `as_str()` / `parse()` round-trip the canonical kebab-case form.
//! `serde(rename_all = "kebab-case")` matches that representation —
//! a future ticket serialization that includes `Flavor` directly will
//! produce the same bytes as the current `resource_class: "small"`.

#[allow(unused_imports)]
use alloc::string::String;

use serde::{Deserialize, Serialize};

/// Canonical tenant VM flavor catalogue.
///
/// Each variant is a closed (vcpus, memory_mb, disk_gb) tuple. The
/// catalogue is intentionally small — every new variant requires:
///
/// 1. a §22 allowlist entry per (flavor, image) pair (vCPU count is
///    measurement-affecting under SEV-SNP — see
///    [`crate::ticket::OrderTicket::allowed_measurements`]);
/// 2. a baked image whose qcow2 size matches `disk_gb()` (the bake
///    script's `--output-qcow2-gb` flag in `scripts/tenant-image-
///    bake.sh`);
/// 3. an audit of whether the operator's miners actually offer the
///    requested capacity (the miner-agent doesn't gate this today,
///    but it will once it learns to verify the ticket's flavor
///    against the launch payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Flavor {
    /// 1 vCPU, 2048 MiB RAM, 8 GiB disk.
    Small,
    /// 2 vCPU, 4096 MiB RAM, 16 GiB disk.
    Medium,
    /// 4 vCPU, 8192 MiB RAM, 32 GiB disk.
    Large,
    /// 8 vCPU, 16384 MiB RAM, 64 GiB disk.
    Xlarge,
    /// 16 vCPU, 32768 MiB RAM, 128 GiB disk.
    #[serde(rename = "2xlarge")]
    X2large,
    /// 32 vCPU, 65536 MiB RAM, 256 GiB disk.
    #[serde(rename = "4xlarge")]
    X4large,
}

impl Flavor {
    /// Number of vCPUs the guest is launched with.
    ///
    /// **Measurement-affecting** — folded into the SEV-SNP launch
    /// digest via QEMU's `-smp` flag. A different vCPU count produces
    /// a different `launch_digest_hex`; each flavor therefore needs
    /// its own §22 allowlist entry per kernel+initrd+cmdline triple.
    pub const fn vcpus(self) -> u8 {
        match self {
            Self::Small => 1,
            Self::Medium => 2,
            Self::Large => 4,
            Self::Xlarge => 8,
            Self::X2large => 16,
            Self::X4large => 32,
        }
    }

    /// Guest RAM in MiB. NOT measurement-affecting (QEMU's `-m`
    /// argument doesn't fold into the SNP measurement), so memory can
    /// vary per launch without rotating the allowlist.
    pub const fn memory_mb(self) -> u32 {
        match self {
            Self::Small => 2048,
            Self::Medium => 4096,
            Self::Large => 8192,
            Self::Xlarge => 16384,
            Self::X2large => 32768,
            Self::X4large => 65536,
        }
    }

    /// Tenant rootfs size in GiB. The baked qcow2 must be sized to
    /// match — `tenant-image-bake.sh --output-qcow2-gb` MUST equal
    /// this value. Disk size doesn't enter the SNP measurement
    /// directly, but `luks_disk_sha256_hex` (a separate ticket-side
    /// pin) does: a 32 GiB qcow2 and an 8 GiB qcow2 carry different
    /// LUKS keyslot offsets and different SHA-256s.
    pub const fn disk_gb(self) -> u32 {
        match self {
            Self::Small => 8,
            Self::Medium => 16,
            Self::Large => 32,
            Self::Xlarge => 64,
            Self::X2large => 128,
            Self::X4large => 256,
        }
    }

    /// Canonical kebab-case identifier. This is the EXACT string that
    /// goes into the ticket's `resource_class` field today, and the
    /// string `Flavor` Serialize emits via `rename_all = "kebab-case"`
    /// for future ticket-schema migrations.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Xlarge => "xlarge",
            Self::X2large => "2xlarge",
            Self::X4large => "4xlarge",
        }
    }

    /// Iterate the catalogue in declaration order. Useful for CLI
    /// help text + completion + reflection tests.
    pub const fn all() -> &'static [Self] {
        &[
            Self::Small,
            Self::Medium,
            Self::Large,
            Self::Xlarge,
            Self::X2large,
            Self::X4large,
        ]
    }
}

/// Errors surfaced by [`Flavor::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlavorParseError {
    /// The input string was not one of the canonical kebab-case
    /// identifiers in [`Flavor::all`]. Carries the unrecognised input
    /// for the caller to surface in operator-facing error text.
    Unknown(String),
}

impl core::fmt::Display for FlavorParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown(s) => write!(
                f,
                "unknown flavor {s:?} — expected one of: small, medium, large, xlarge, 2xlarge, 4xlarge"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FlavorParseError {}

impl core::str::FromStr for Flavor {
    type Err = FlavorParseError;

    fn from_str(s: &str) -> core::result::Result<Self, Self::Err> {
        match s {
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            "xlarge" => Ok(Self::Xlarge),
            "2xlarge" => Ok(Self::X2large),
            "4xlarge" => Ok(Self::X4large),
            other => Err(FlavorParseError::Unknown(other.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::str::FromStr;

    #[test]
    fn vcpus_memory_disk_match_declared_doc_comments() {
        // Pin the catalogue values so a doc-comment edit can't drift
        // from the runtime numbers without the test catching it.
        assert_eq!(Flavor::Small.vcpus(), 1);
        assert_eq!(Flavor::Small.memory_mb(), 2048);
        assert_eq!(Flavor::Small.disk_gb(), 8);

        assert_eq!(Flavor::Medium.vcpus(), 2);
        assert_eq!(Flavor::Medium.memory_mb(), 4096);
        assert_eq!(Flavor::Medium.disk_gb(), 16);

        assert_eq!(Flavor::Large.vcpus(), 4);
        assert_eq!(Flavor::Large.memory_mb(), 8192);
        assert_eq!(Flavor::Large.disk_gb(), 32);

        assert_eq!(Flavor::Xlarge.vcpus(), 8);
        assert_eq!(Flavor::Xlarge.memory_mb(), 16384);
        assert_eq!(Flavor::Xlarge.disk_gb(), 64);

        assert_eq!(Flavor::X2large.vcpus(), 16);
        assert_eq!(Flavor::X2large.memory_mb(), 32768);
        assert_eq!(Flavor::X2large.disk_gb(), 128);

        assert_eq!(Flavor::X4large.vcpus(), 32);
        assert_eq!(Flavor::X4large.memory_mb(), 65536);
        assert_eq!(Flavor::X4large.disk_gb(), 256);
    }

    #[test]
    fn as_str_round_trips_via_from_str() {
        for &f in Flavor::all() {
            assert_eq!(Flavor::from_str(f.as_str()), Ok(f));
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert_eq!(
            Flavor::from_str("huge"),
            Err(FlavorParseError::Unknown("huge".into()))
        );
        assert_eq!(
            // "8xlarge" is not in the catalogue (top is 4xlarge).
            Flavor::from_str("8xlarge"),
            Err(FlavorParseError::Unknown("8xlarge".into()))
        );
        assert_eq!(
            Flavor::from_str("Small"), // Wrong case — kebab-case only.
            Err(FlavorParseError::Unknown("Small".into()))
        );
        assert_eq!(
            Flavor::from_str(""),
            Err(FlavorParseError::Unknown("".into()))
        );
    }

    #[test]
    fn serde_uses_kebab_case_via_ciborium() {
        // The wire format for the ticket is canonical CBOR (RFC 8949
        // §4.2.1) via `ciborium`. A future ticket-schema migration
        // that embeds `Flavor` directly (rather than the `resource_
        // class: String` of today) MUST emit kebab-case strings, so
        // a `Flavor::Small` ⇄ `"small"` round-trip stays byte-exact
        // with the current `resource_class = "small"` ticket bytes.
        let mut buf = alloc::vec::Vec::new();
        ciborium::ser::into_writer(&Flavor::Small, &mut buf).unwrap();
        // CBOR text-string of length 5: 0x65 'small'.
        assert_eq!(buf, &[0x65, b's', b'm', b'a', b'l', b'l']);

        // Round-trip.
        let parsed: Flavor = ciborium::de::from_reader(&buf[..]).unwrap();
        assert_eq!(parsed, Flavor::Small);

        // Sanity-check the two other variants — round-trip is enough,
        // we don't need to spell out the exact byte layout for each.
        for &f in Flavor::all() {
            let mut b = alloc::vec::Vec::new();
            ciborium::ser::into_writer(&f, &mut b).unwrap();
            let back: Flavor = ciborium::de::from_reader(&b[..]).unwrap();
            assert_eq!(back, f);
        }
    }

    #[test]
    fn vcpus_are_powers_of_two_for_predictable_smp_topology() {
        // QEMU's `-smp` parses cleanly when vcpus is a power of two
        // (cores/threads decompose without remainder). The check also
        // surfaces accidental "3 vcpu" entries that look fine in
        // isolation but produce uneven NUMA pinning on multi-socket
        // hosts.
        for &f in Flavor::all() {
            let n = u32::from(f.vcpus());
            assert!(
                n.is_power_of_two(),
                "{f:?}.vcpus() = {n} — must be a power of two for predictable SMP topology"
            );
        }
    }
}
