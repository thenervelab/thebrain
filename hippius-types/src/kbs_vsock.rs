//! Wire contract for the **KBS-release-over-vsock relay** — the
//! permissionless replacement for the tenant guest reaching the KBS
//! over the network directly.
//!
//! ## Why
//!
//! `hippius-guest-release` runs the §21 release exchange inside the
//! tenant CVM and, today, POSTs `/v1/kbs/nonce` + `/v1/kbs/release`
//! over **direct HTTP** to `https://kbs.hippius.network`. That KBS
//! address resolves to an **in-cluster / mesh-only** IP: the
//! guest's forwarded traffic cannot reach it through the miner's
//! NetBird mesh (only the miner's own host traffic can). Routing the
//! guest's KBS traffic through the host's mesh is fragile — it broke
//! the moment NetBird's routing drifted.
//!
//! The robust model (the §102 "kata cannot reach VIPs → relay" lesson):
//! the guest does **zero** network for the KBS. It connects over
//! **AF_VSOCK** to the miner-agent on the host, which proxies the two
//! KBS POSTs to the real KBS over the host's network and returns the
//! responses. The guest needs no DNS, no route, no mesh — only the
//! host vsock channel it already uses for the OrderTicket.
//!
//! ## Wire
//!
//! One request → one response per connection, each a CBOR value with a
//! `u32` big-endian length prefix (the same framing as
//! [`crate::ticket_vsock`] and the guest→Edge relay):
//!
//! - guest → host: [`KbsProxyRequest`] `{ path, body }`
//! - host → guest: [`KbsProxyResponse`] `{ status, body }`
//!
//! The host MUST restrict `path` to the allow-listed endpoints (see
//! [`is_allowed_path`]) so the proxy can never be coerced into an
//! arbitrary-URL SSRF.
//!
//! ## Two forwarded destinations (§25 source-ack reuses this channel)
//!
//! The same vsock proxy carries two distinct flows, routed by `path`:
//!
//! - the §21 KBS release exchange ([`KBS_NONCE_PATH`] /
//!   [`KBS_RELEASE_PATH`]) → forwarded to the **KBS**, and
//! - the §24/§25 guest stopped-ack push ([`LIFECYCLE_STOPPED_PATH`]) →
//!   forwarded to **vali**'s `/v1/lifecycle/stopped` ingress.
//!
//! The confidential guest has no IP route to either in-cluster service,
//! so both ride the one host vsock channel. The stopped-ack carries a
//! `?vm_id=&generation=` query the host forwards verbatim — see
//! [`is_allowed_path`], which matches on the path component only (the
//! query never widens the allow-list).

// `String` comes from `alloc`, not `std`: this crate is
// `#![cfg_attr(not(feature = "std"), no_std)]` because
// `pallet-compute-scoring` depends on it with `default-features = false`
// and has to compile into a wasm32 Substrate runtime. Every other module
// here already imports its alloc types explicitly; this one did not, so
// the crate stopped building for `wasm32-unknown-unknown` — invisibly,
// since a host-target build resolves `String` from the std prelude.
use alloc::string::String;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

/// AF_VSOCK port the miner-agent listens on for the KBS proxy. Distinct
/// from [`crate::ticket_vsock::PORT`] (host→guest ticket push) and the
/// guest→Edge relay port. `0x4B42` spells "KB" (KBS).
pub const PORT: u32 = 0x4B42;

/// The well-known host context-id the guest dials (`VMADDR_CID_HOST`).
pub const HOST_CID: u32 = 2;

/// Hard cap on a single proxied request body — the §21 release request
/// (cose_ticket + snp_report + nonce) is a few KB; this bounds a
/// hostile guest's allocation before any work.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Hard cap on a single proxied response body — the KBS release
/// response (HPKE-wrapped KEK + signature) is small.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Connect/exchange timeout for one proxied request, seconds.
pub const PROXY_TIMEOUT_SECS: u64 = 35;

/// The §21 KBS nonce endpoint path.
pub const KBS_NONCE_PATH: &str = "/v1/kbs/nonce";

/// The §21 KBS release endpoint path.
pub const KBS_RELEASE_PATH: &str = "/v1/kbs/release";

/// The volume-stamp CONFIRM endpoint on the KBS. After unlocking its
/// guest-keyed volume the guest stamps it and POSTs the single-use
/// authenticator here; only that confirmation advances the KBS's stored
/// expectation, which is what stops an ABORTED boot from widening the
/// expected-vs-actual gap (the accumulation that would otherwise let a
/// host brick a tenant by repeatedly killing the VM post-release).
///
/// It MUST be in [`ALLOWED_PATHS`]: production guests reach the KBS over
/// vsock (`hippius.kbs_url=vsock://2:19266`, verified on the live tenant),
/// so a confirm left off this list fails closed with `vsock-path-forbidden`
/// on every real boot while every unit test still passes.
pub const KBS_VOLUME_STAMP_CONFIRM_PATH: &str = "/v1/kbs/volume-stamp/confirm";

/// The §24/§25 guest stopped-ack ingress path on vali. The guest POSTs
/// the opaque `SignedStoppedAck` here with a `?vm_id=&generation=`
/// query (which the proxy forwards verbatim). vali stores the bytes
/// keyed by `(vm_id, generation)`; its orchestrator polls + verifies
/// them later — the proxy is a pure transport (§5.6 opacity).
pub const LIFECYCLE_STOPPED_PATH: &str = "/v1/lifecycle/stopped";

/// The §23 keepalive live-attestation endpoint on the KBS. The tenant
/// guest's `hippius-agent-keepalive` daemon POSTs a fresh SNP report
/// here every tick; the KBS verifies it against AMD silicon + the §22
/// allowlist and returns a KBS-L0-signed `SignedLiveAttestation` that
/// vali's uptime-coverage meter credits.
///
/// Same fail-closed asymmetry as [`KBS_VOLUME_STAMP_CONFIRM_PATH`]: the
/// baked tenant image reaches the KBS over vsock
/// (`hippius.kbs_url=vsock://2:19266`), so leaving this off
/// [`ALLOWED_PATHS`] would make EVERY real boot's keepalive fail with
/// `vsock-path-forbidden` — no `VmLiveAttestation` row would ever land,
/// the uptime gate could never be armed — while every https-based unit
/// test still passed.
pub const KBS_KEEPALIVE_PATH: &str = "/v1/attest/keepalive";

/// The paths the proxy is allowed to forward — matched against the
/// **path component only** (any `?query` is stripped before the check;
/// see [`is_allowed_path`]). Restricting this is load-bearing: it stops
/// the vsock proxy from being an open SSRF into the host's network.
///
/// - the two §21 KBS release endpoints (forwarded to the KBS),
/// - the volume-stamp confirm (forwarded to the KBS),
/// - the §23 keepalive live-attestation mint (forwarded to the KBS), and
/// - the §24/§25 guest stopped-ack ingress (forwarded to vali).
pub const ALLOWED_PATHS: &[&str] = &[
    KBS_NONCE_PATH,
    KBS_RELEASE_PATH,
    KBS_VOLUME_STAMP_CONFIRM_PATH,
    KBS_KEEPALIVE_PATH,
    LIFECYCLE_STOPPED_PATH,
];

/// Split a request `path` into its path component and the raw query
/// string (the part after the first `?`, EXCLUSIVE of the `?`; empty
/// when there is none). The query is never matched against the
/// allow-list — only the path component is — but it IS forwarded
/// verbatim so vali's ingress sees `?vm_id=&generation=`.
pub fn split_query(path: &str) -> (&str, &str) {
    match path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path, ""),
    }
}

/// `true` iff `path` (ignoring any `?query`) is an endpoint the proxy
/// may forward. A query string can never widen the allow-list: only the
/// stopped-ack path is expected to carry one, and it is matched on its
/// path component alone.
pub fn is_allowed_path(path: &str) -> bool {
    let (component, _query) = split_query(path);
    ALLOWED_PATHS.contains(&component)
}

/// `true` iff `path` (ignoring any `?query`) targets vali's lifecycle
/// stopped-ack ingress rather than the KBS — the proxy routes a `true`
/// to its vali backend and everything else (the KBS paths) to the KBS
/// backend.
pub fn is_lifecycle_path(path: &str) -> bool {
    let (component, _query) = split_query(path);
    component == LIFECYCLE_STOPPED_PATH
}

/// Guest → host: "POST this body to `<kbs_url><path>` and give me the
/// response." `path` is the KBS endpoint path (e.g. `/v1/kbs/nonce`);
/// the host supplies the KBS base URL from its own config — the guest
/// never names a host or IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KbsProxyRequest {
    /// KBS endpoint path (must be in [`ALLOWED_PATHS`]).
    pub path: String,
    /// The canonical-CBOR request body, posted verbatim
    /// (`application/cbor`). Empty for the nonce endpoint.
    pub body: ByteBuf,
}

/// Host → guest: the KBS's HTTP response, relayed verbatim. `status`
/// is the real KBS HTTP status (200 / 403 / …) so the guest's existing
/// `kbs_client` status handling is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KbsProxyResponse {
    /// The KBS HTTP status code.
    pub status: u16,
    /// The KBS response body, verbatim.
    pub body: ByteBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allowed_paths_are_forwarded() {
        assert!(is_allowed_path("/v1/kbs/nonce"));
        assert!(is_allowed_path("/v1/kbs/release"));
        // The volume-stamp confirm. Production guests reach the KBS over
        // vsock, so leaving this off the list fails EVERY real boot's
        // confirm closed with `vsock-path-forbidden` — while every unit
        // test that drives the confirm over https still passes. That
        // asymmetry is exactly why this assertion exists.
        assert!(is_allowed_path("/v1/kbs/volume-stamp/confirm"));
        assert!(is_allowed_path(KBS_VOLUME_STAMP_CONFIRM_PATH));
        // The §23 keepalive mint. The baked tenant image runs
        // `hippius-agent-keepalive` against a `vsock://` KBS URL, so an
        // absent entry here fails EVERY real boot's keepalive closed with
        // `vsock-path-forbidden` — no VmLiveAttestation ever lands and the
        // uptime gate can never be armed — while every https-based test
        // still passes. Same asymmetry as the volume-stamp confirm above.
        assert!(is_allowed_path("/v1/attest/keepalive"));
        assert!(is_allowed_path(KBS_KEEPALIVE_PATH));
        // …and it routes to the KBS backend, not vali's lifecycle ingress.
        assert!(!is_lifecycle_path(KBS_KEEPALIVE_PATH));
        // The §25 stopped-ack ingress — with and without its query.
        assert!(is_allowed_path("/v1/lifecycle/stopped"));
        assert!(is_allowed_path(
            "/v1/lifecycle/stopped?vm_id=vm-x&generation=5"
        ));
        // Anything else — an SSRF attempt — is refused.
        assert!(!is_allowed_path("/v1/kbs/evidence"));
        assert!(!is_allowed_path("/v1/admin/allowlist/reload"));
        assert!(!is_allowed_path("http://169.254.169.254/"));
        assert!(!is_allowed_path("/"));
        assert!(!is_allowed_path(""));
        // A query can never widen the allow-list: a forbidden path with
        // a benign-looking query is still refused.
        assert!(!is_allowed_path(
            "/v1/admin/allowlist/reload?vm_id=x&generation=1"
        ));
    }

    #[test]
    fn split_query_separates_path_and_query() {
        assert_eq!(
            split_query("/v1/lifecycle/stopped?vm_id=vm-x&generation=5"),
            ("/v1/lifecycle/stopped", "vm_id=vm-x&generation=5")
        );
        assert_eq!(split_query("/v1/kbs/nonce"), ("/v1/kbs/nonce", ""));
        // A bare trailing '?' yields an empty query, not a missing one.
        assert_eq!(split_query("/p?"), ("/p", ""));
    }

    #[test]
    fn lifecycle_path_is_distinguished_from_kbs_paths() {
        assert!(is_lifecycle_path("/v1/lifecycle/stopped"));
        assert!(is_lifecycle_path(
            "/v1/lifecycle/stopped?vm_id=x&generation=2"
        ));
        assert!(!is_lifecycle_path("/v1/kbs/nonce"));
        assert!(!is_lifecycle_path("/v1/kbs/release"));
        // The confirm carries a KBS-issued authenticator and MUST route
        // to the KBS backend, never to vali's lifecycle ingress.
        assert!(!is_lifecycle_path(KBS_VOLUME_STAMP_CONFIRM_PATH));
    }

    #[test]
    fn request_round_trips_cbor() {
        let req = KbsProxyRequest {
            path: "/v1/kbs/release".to_string(),
            body: ByteBuf::from(vec![1u8, 2, 3]),
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&req, &mut buf).unwrap();
        let back: KbsProxyRequest = ciborium::de::from_reader(&buf[..]).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn response_round_trips_cbor() {
        let resp = KbsProxyResponse {
            status: 200,
            body: ByteBuf::from(vec![9u8; 48]),
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&resp, &mut buf).unwrap();
        let back: KbsProxyResponse = ciborium::de::from_reader(&buf[..]).unwrap();
        assert_eq!(resp, back);
    }
}
