//! #312/#365 — name-parity gate between the Rust `Flavor` enum and
//! `scripts/tenant-image-bake.sh`'s `--flavor` resolution.
//!
//! **History.** #312 made the bake size the rootfs to the flavor's
//! `disk_gb`, and this gate pinned the bash `output_qcow2_gb` table to
//! the Rust `disk_gb()`. #365 removed that coupling: a LUKS2 +
//! `--integrity` volume cannot be grown (`cryptsetup resize` refuses an
//! integrity-protected device), so the rootfs is now a fixed,
//! flavor-INDEPENDENT minimal image and the flavor's `disk_gb` sizes a
//! SEPARATE tenant data disk the miner attaches at `/dev/vde`. The bake
//! no longer encodes `disk_gb` at all.
//!
//! What still must not drift is the set of **accepted flavor names**: a
//! variant added to the Rust enum but not accepted by the bake's
//! `--flavor` validation would make `vali_create_vm --flavor newname`
//! fail at bake time. And the dead per-flavor sizing must not creep
//! back. This test guards both.
//!
//! ```text
//! cargo test -p hippius-types --test flavor_bake_catalogue
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use hippius_types::flavor::Flavor;
use std::path::PathBuf;

/// Locate `scripts/tenant-image-bake.sh` relative to the workspace
/// root. `CARGO_MANIFEST_DIR` points at `hippius-types/`; the bake
/// script is one level up.
fn bake_script_path() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let mut p = PathBuf::from(manifest);
    p.pop(); // → workspace root
    p.push("scripts");
    p.push("tenant-image-bake.sh");
    p
}

fn bake_script_src() -> String {
    let path = bake_script_path();
    assert!(
        path.exists(),
        "bake script not found at {} — workspace layout drift?",
        path.display()
    );
    std::fs::read_to_string(&path).expect("read bake script")
}

/// The single `case "${flavor}" in … esac` arm that validates
/// `--flavor`. We slice it out so substring checks below can't match
/// an unrelated mention of a flavor name elsewhere in the script.
fn flavor_case_arm(src: &str) -> String {
    // The accepted-name arm collapses every variant onto one pattern
    // line ending in `) ;;` — find the line that lists `small` and
    // `4xlarge` together (the canonical accepted set, #365).
    src.lines()
        .find(|l| l.contains("small") && l.contains("4xlarge") && l.contains(") ;;"))
        .unwrap_or_else(|| {
            panic!("could not find the collapsed --flavor accept arm in the bake script")
        })
        .to_string()
}

#[test]
fn every_rust_variant_is_accepted_by_the_bake() {
    let src = bake_script_src();
    let arm = flavor_case_arm(&src);
    for &f in Flavor::all() {
        // Each name appears as a `|name|`/`name)` token in the arm.
        // Match with a trailing delimiter so `xlarge` doesn't spuriously
        // satisfy `2xlarge`/`4xlarge` (and vice-versa we still want each
        // present explicitly).
        let name = f.as_str();
        let present = arm.contains(&format!("|{name})"))
            || arm.contains(&format!("|{name}|"))
            || arm.contains(&format!("\"|{name}|"))
            || arm.contains(&format!("\"\"|{name}|"));
        assert!(
            present,
            "Flavor::{f:?} ({name}) is not an accepted --flavor value in the bake: arm=`{arm}`"
        );
    }
}

#[test]
fn bake_does_not_size_the_rootfs_by_flavor() {
    // #365 guard: resurrecting a per-flavor `output_qcow2_gb=<N>` arm
    // would bring back the dead "bake the full flavor disk then grow"
    // path. No line that names a flavor variant may also assign
    // `output_qcow2_gb`.
    let src = bake_script_src();
    for line in src.lines() {
        let names_a_flavor = Flavor::all()
            .iter()
            .any(|f| line.contains(&format!("{})", f.as_str())));
        if names_a_flavor {
            assert!(
                !line.contains("output_qcow2_gb="),
                "a --flavor arm still sizes the rootfs (#365 dead path): `{line}`"
            );
        }
    }
}
