//! Wire types for the KBS admin API (ARCHITECTURE.md §24/§25 lifecycle
//! pre-registration). Consumed by `kbs-admin-client` (the Rust subprocess
//! vali invokes before dispatch) and by `kbs-core::admin` (the verifier).
//!
//! ## Scope
//!
//! The request body for `POST /v1/admin/register-vm` is the byte-exact
//! signed COSE_Sign1 `OrderTicket` envelope vali already submits to
//! `POST /v1/order_ticket`. **No bespoke request struct.** This keeps:
//!
//! - The cryptographic anchor identical to the existing `verify_order_
//!   ticket` path (kbs-core derives `vm_id`, `vm_generation`,
//!   `platform_id`, `lease_id` from the verified ticket, not from
//!   anything vali sends).
//! - Idempotency keyed by the ticket's `ticket_id` (durably
//!   single-use across retries).
//! - The §24 lifecycle gate's input shape unchanged from the
//!   existing §6 ticket schema — no parallel CBOR types to keep in
//!   sync.
//!
//! ## Scope
//!
//! `register-vm` ships the verified-ticket write. `activate` (§25 M2 —
//! the destination re-activation fence, `Active → Migrating`) ships a
//! JSON request ([`AdminActivateRequest`]) + response
//! ([`AdminActivateResponse`]). The `decommission` / `crypto-erase` ops
//! referenced by `vali/apps/orchestration/effects.py` (§24) remain
//! follow-ups; their handlers return 501 until implemented. `AdminOp`
//! carries the full enum today so the wire shape is locked at v1.

#[allow(unused_imports)]
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use serde::{Deserialize, Serialize};

/// The admin operation discriminator. Mirrors the URL path component
/// vali already uses in `effects.py:_kbs_post(vm, command, …)` — same
/// names, same case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdminOp {
    /// First-time write of a `VmState::Active`. CAS predicate:
    /// `cur.is_none() || cur == Some(Active { gen, host, lease_id })`.
    /// Idempotent on identical state.
    RegisterVm,
    /// Phase B — Active → Decommissioning. Currently 501.
    Decommission,
    /// Phase B — Decommissioning → Destroyed{gen}. Currently 501.
    CryptoErase,
    /// §25 M2 — Active → Migrating{new_gen, dest}: the destination
    /// re-activation fence. Body is [`AdminActivateRequest`].
    Activate,
    /// Operator disaster recovery — restore a WIPED per-VM boot counter
    /// to the value recovered from the miner-side state disk. Refuses
    /// (nothing written) unless the stored counter is absent/0, refuses
    /// an implausibly large value, and is strictly monotonic. Body is
    /// [`AdminSeedBootCounterRequest`].
    SeedBootCounter,
    /// Operator recovery for the suppressed-confirm anti-rollback gate
    /// (`kbs_core::volume_stamp`) — clear a `vm_id`'s unconfirmed-release
    /// counter WITHOUT touching its confirmed volume stamp. A miner can
    /// drop every `/v1/kbs/volume-stamp/confirm` it relays; the KBS
    /// detects that (via `note_release` at every release) and refuses
    /// further releases once the count exceeds
    /// `kbs_core::volume_stamp::MAX_UNCONFIRMED_RELEASES`, so the VM
    /// cannot be silently rolled back — but recovering it can ONLY be an
    /// authenticated admin action (this route), never anything the guest
    /// or the miner could trigger. Empty request body; response is
    /// [`AdminResetVolumeStampSuppressionResponse`].
    ResetVolumeStampSuppression,
    /// Operator disaster recovery for the OTHER direction of boot-counter
    /// loss from [`AdminOp::SeedBootCounter`]: the GUEST's copy is gone
    /// (the miner-side state disk was lost) while the KBS's is intact.
    ///
    /// Arms a ONE-SHOT resync. It writes no counter: the next release
    /// for this VM that the strict `stored + 1` CAS would refuse is
    /// admitted once, and commits `stored + 1` — never the value the
    /// guest submitted — so the counter still only ever moves up by one
    /// and no burned boot is re-admitted. The guest re-learns the value
    /// from the signed release response. Empty request body; response is
    /// [`AdminArmBootCounterResyncResponse`].
    ArmBootCounterResync,
}

impl AdminOp {
    /// URL path component (the last segment of
    /// `/v1/admin/{op-name}`).
    pub fn path_segment(self) -> &'static str {
        match self {
            AdminOp::RegisterVm => "register-vm",
            AdminOp::Decommission => "decommission",
            AdminOp::CryptoErase => "crypto-erase",
            AdminOp::Activate => "activate",
            AdminOp::SeedBootCounter => "seed-boot-counter",
            AdminOp::ResetVolumeStampSuppression => "reset-volume-stamp-suppression",
            AdminOp::ArmBootCounterResync => "arm-boot-counter-resync",
        }
    }
}

/// Request body for `POST /v1/admin/vm/{vm_id}/seed-boot-counter`.
///
/// The KBS keeps its boot counters in a state dir that does not survive
/// a pod restart (`emptyDir` on a Kata CVM). After a wipe, every VM
/// that has already booted is locked out — it submits `N + 1` while the
/// empty store expects `1`. The authoritative value survives on the
/// miner-side per-VM state disk; the operator posts it here.
///
/// It does exactly this one thing. The KBS refuses, writing nothing,
/// unless the stored counter is absent/0 (409 `seed-already-recovered`
/// — so it is inert against a live VM, and re-running a recovery
/// refuses rather than overwriting); it refuses an implausibly large
/// value (400 `seed-above-cap` — otherwise one request could set a
/// counter the guest can never reach, bricking the disk); and it applies
/// only upward (409 `seed-not-monotonic`), so it can never re-admit a
/// burned boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSeedBootCounterRequest {
    /// The recovered counter — the number of boots the KBS should
    /// consider already consumed. Must be `>= 1`, at most the KBS's
    /// `MAX_SEED_COUNTER`, and the KBS must currently hold no counter
    /// for this `vm_id`.
    pub counter: u64,
}

/// 200 OK body for `POST /v1/admin/vm/{vm_id}/seed-boot-counter`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSeedBootCounterResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// The VM whose counter was seeded (echoed from the URL).
    pub vm_id: String,
    /// What the KBS held before this call. Always `0` — the KBS only
    /// seeds an absent row — echoed so the operator sees confirmation
    /// that this really was a post-wipe recovery.
    pub previous: u64,
    /// The value now stored. The guest's next boot must submit
    /// `counter + 1`.
    pub counter: u64,
}

/// 200 OK body for `POST /v1/admin/vm/{vm_id}/reset-volume-stamp-suppression`.
///
/// The request body is empty — the URL's `{vm_id}` is the only input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminResetVolumeStampSuppressionResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// The VM whose suppression counter was reset (echoed from the URL).
    pub vm_id: String,
    /// The unconfirmed-release count that was cleared. `0` means the VM
    /// was not blocked — a harmless no-op, not an error (mirrors
    /// `kbs_core::volume_stamp::VolumeStampStore::admin_reset_unconfirmed`).
    pub cleared: u64,
}

/// 200 OK body for `POST /v1/admin/vm/{vm_id}/arm-boot-counter-resync`.
///
/// The request body is empty — the URL's `{vm_id}` is the only input.
/// Note what is NOT in this shape: any counter the operator gets to
/// choose. The re-baseline target is the KBS's own `stored + 1`, so
/// there is no number here to get wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminArmBootCounterResyncResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// The VM whose resync was armed (echoed from the URL).
    pub vm_id: String,
    /// The counter the KBS holds — UNCHANGED by this call. The next
    /// boot will be re-baselined to `stored + 1`.
    pub stored: u64,
    /// `true` when this VM was already armed and the call wrote
    /// nothing new.
    pub already_armed: bool,
}

/// One VM's row in [`AdminVolumeStampReportResponse`].
///
/// Both raw counters AND both derived verdicts are carried. The raw
/// numbers so the operator can see the state; the verdicts so the
/// arithmetic that decides them (in particular gate 5c's
/// increment-then-compare off-by-one) lives in the KBS, where it is
/// tested against the gate itself, rather than in whatever `jq`
/// expression an operator writes at 3am.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminVolumeStampRow {
    /// The VM this row belongs to.
    pub vm_id: String,
    /// Last CONFIRMED volume stamp. `0` = never confirmed.
    pub confirmed: u64,
    /// Releases granted since the last confirm / admin reset.
    pub unconfirmed_releases: u64,
    /// `confirmed > 0` — this VM's guest has landed at least one
    /// `/v1/kbs/volume-stamp/confirm` through its miner. `false` means
    /// the anti-rollback expectation for this VM is frozen at 0 and the
    /// gate is, for it, inert.
    pub has_ever_confirmed: bool,
    /// `true` ⇒ arming at `evaluated_bound` refuses this VM's very next
    /// release (next boot, §25 migration, or reboot-recovery relaunch).
    pub would_refuse_next_release: bool,
}

/// 200 OK body for `GET /v1/admin/volume-stamp?bound=N` — the READ side
/// of the suppressed-confirm anti-rollback gate.
///
/// This exists to make the chart's arming cutover step 3 ("verify
/// confirms are arriving fleet-wide") an actual check. Before it, every
/// path that touched `kbs_core::volume_stamp` was a WRITE — the guest's
/// confirm, the release path's `note_release`, the admin suppression
/// reset — and the store lives on an emptyDir inside a Kata CVM where
/// `kubectl exec` does not work, so the operator was asked to verify a
/// condition nothing could observe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminVolumeStampReportResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// The bound the rows were evaluated against — the value the
    /// operator is PROPOSING to arm at (`?bound=`, default = the
    /// binary's compiled `MAX_UNCONFIRMED_RELEASES`).
    pub evaluated_bound: u64,
    /// The bound this KBS process is ACTUALLY running with. `None` ⇒
    /// the gate is DISABLED (`storage.max_unconfirmed_releases = 0`).
    /// Reported separately from `evaluated_bound` so a green report can
    /// never be mistaken for "the gate is on and happy".
    pub configured_bound: Option<u64>,
    /// `configured_bound.is_some()` — restated as a boolean so the
    /// single most important fact about this KBS is not encoded as the
    /// absence of a JSON field.
    pub gate_armed: bool,
    /// How many VMs the store knows about.
    pub vms: usize,
    /// Count of rows with `has_ever_confirmed == false`.
    pub never_confirmed: usize,
    /// Count of rows with `would_refuse_next_release == true`.
    pub would_refuse_now: usize,
    /// `never_confirmed == 0 && would_refuse_now == 0`. Cutover step 3
    /// in one boolean.
    pub ready_to_arm: bool,
    /// Every VM the KBS has ever granted a release for, sorted by
    /// `vm_id`. A row is created by `note_release`, so this is the fleet
    /// as the KBS sees it — not an opt-in subset.
    pub rows: Vec<AdminVolumeStampRow>,
}

/// 200 OK body for `GET /v1/admin/config` — the EFFECTIVE security
/// posture of the RUNNING KBS process.
///
/// ## Why this exists
///
/// `Config::load` runs exactly ONCE, at
/// `binaries/kbs-server/src/main.rs`, and the KBS deployment carries no
/// `checksum/config` annotation (deliberately — rolling the pod wipes
/// the `state`/`audit`/`evidence` emptyDirs inside its Kata CVM). So a
/// config change lands in the ConfigMap and the running process keeps
/// its old value INDEFINITELY, while ArgoCD reports `Synced` +
/// `Healthy`. On 2026-08-13 the suppressed-confirm anti-rollback gate
/// was raised `0 → 3`, merged and synced; the rendered ConfigMap read
/// `max_unconfirmed_releases = 3` while the 152-minute-old process was
/// still running with the gate DISABLED. Only asking the process itself
/// caught it.
///
/// `GET /v1/admin/volume-stamp` already answers that one question
/// (`configured_bound`). This is the generalisation: one read-only
/// readout of what the process ACTUALLY resolved at startup, so a
/// monitor can diff intent against reality for the whole security
/// posture rather than for one key.
///
/// Every field is computed from the SAME `Config` value the release
/// path was wired from, at wiring time
/// (`hippius_kbs_server::wiring::config_posture`) — it is not a re-read
/// of the file and not a second interpretation of the operator's input.
/// A second copy of a decision is a second thing that can drift from it.
///
/// ## ⛔ Posture, never material
///
/// This body sits behind the admin mTLS listener and is additionally
/// refused unless the request arrived over a VERIFIED client
/// certificate — but "authenticated" is not a licence to publish
/// secrets. The rule for every field here is: **booleans, bounds,
/// counts, enum labels and fingerprints only.**
///
/// Deliberately ABSENT, and they must stay absent:
/// - every filesystem path (signing key, TLS material, VEK, state dirs)
///   — a path is a map to where a secret lives;
/// - every URL (`vault.address`, `vault.broker_url`, listen addresses)
///   — those disclose topology and the location of the KEK custodian;
/// - `vault.kv_mount`, which names where KEKs are stored;
/// - raw public keys. `allowlist.root_pubkey_hex` is public and already
///   in Git, but a FINGERPRINT answers the only question a monitor asks
///   ("is the process anchored to the key I think?") without turning
///   this endpoint into a key-distribution surface;
/// - `keys.auth_pubkey_hex` and the `[live_attestation]` chain
///   discriminators: arguably public, genuinely unsure what an
///   adversary does with them, so LEFT OUT (see the PR description).
///
/// `hippius_types::admin::tests::config_posture_exposes_exactly_the_
/// pinned_field_set` pins the serialised key set, so a future field
/// addition has to be argued for in that test rather than slipped in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminConfigPostureResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// KEK-HSM RA-08a/F2 — `true` ⇒ this process REFUSES a
    /// non-Transit-wrapped (plaintext) KEK at release. `#[serde(default)]`
    /// on the config side means an omitted key silently reads `false`,
    /// which is exactly the kind of change this endpoint exists to make
    /// observable.
    pub require_wrapped_kek: bool,
    /// The RESOLVED suppressed-confirm bound (`kbs_core::volume_stamp`)
    /// gate 5c is enforcing. `None` ⇒ the gate is DISABLED. Identical
    /// to [`AdminVolumeStampReportResponse::configured_bound`] — same
    /// `Option<u64>`, same resolution function.
    pub max_unconfirmed_releases: Option<u64>,
    /// `max_unconfirmed_releases.is_some()`, restated so the single
    /// most important fact is not encoded as an absent JSON value.
    pub volume_stamp_gate_armed: bool,
    /// The admin listener mode `AdminListenerMode::decide` ACTUALLY
    /// chose for this process: `"mtls"`, `"plaintext-opt-in"` or
    /// `"refuse"`. Not the `require_mtls` flag — that flag is NOT the
    /// enforcement switch (material present enforces mTLS whatever it
    /// says), and confusing the two is precisely how this gets deployed
    /// wrong.
    pub admin_listener_mode: String,
    /// `true` ⇒ a real §280 evidence sink is wired (`storage.evidence_dir`
    /// set). `false` ⇒ `NullEvidenceSink`: every bundle is discarded and
    /// no tenant can ever obtain proof their VM was attested.
    pub evidence_sink_wired: bool,
    /// `true` ⇒ a real §322 live-attestation sink is wired. `false` ⇒
    /// keepalives are served but nothing is archived for the validator,
    /// i.e. attested uptime stops being payable.
    pub live_attestation_sink_wired: bool,
    /// Fingerprint of the §22 allowlist root pubkey this process
    /// verifies allowlist artifacts against: the first 8 bytes of
    /// `SHA-256(pubkey_bytes)`, lower-case hex (16 chars). Reproducible
    /// with `echo -n <hex> | xxd -r -p | sha256sum | cut -c1-16`.
    /// `"unparseable"` if the configured hex did not decode.
    pub allowlist_root_pubkey_fpr: String,
    /// Same fingerprint for the OPTIONAL incoming rotation root.
    /// `None` ⇒ single-root steady state (no rotation in flight).
    pub allowlist_root_next_pubkey_fpr: Option<String>,
    /// `true` ⇒ a signed allowlist artifact path is configured. `false`
    /// ⇒ no allowlist can be installed at startup and every release
    /// fails closed.
    pub allowlist_signed_path_configured: bool,
    /// How many L1 OrderTicket-signing keys are loaded. `0` ⇒ every
    /// ticket fails signature verification (fail closed). The kids and
    /// pubkeys themselves are NOT listed — the count is the posture.
    pub l1_key_count: u32,
    /// SNP launch-policy floor — minimum packed TCB version.
    pub min_tcb: u64,
    /// SNP launch-policy bits that MUST be set.
    pub required_bits: u64,
    /// SNP launch-policy bits allowed to vary.
    pub allowed_mask: u64,
    /// `true` ⇒ `[snp]` is configured (a real AMD chain is anchored).
    /// `false` ⇒ the deny-closed chain verifier: every release fails at
    /// the SNP step.
    pub snp_chain_wired: bool,
    /// The static-VEK fallback generation (`"milan"`/`"genoa"`/
    /// `"turin"`), or `None` when `[snp]` is absent. A label, not a path.
    pub snp_generation: Option<String>,
    /// `true` ⇒ AMD KDS fetch is enabled, so a guest report carrying no
    /// cert table can still resolve its VCEK.
    pub snp_kds_fetch_enabled: bool,
    /// `true` ⇒ the SNP-attestation-bound Vault broker is wired (§8 /
    /// #102): the KBS mints a per-VM capability token per release
    /// instead of using a resident static token. The broker URL is NOT
    /// reported.
    pub vault_broker_wired: bool,
    /// `true` ⇒ the broker hop pins a CA (the minted per-VM Vault token
    /// does not transit the pod network in cleartext).
    pub vault_broker_ca_pinned: bool,
    /// `true` ⇒ the Vault KV client verifies the server cert against a
    /// pinned CA bundle.
    pub vault_ca_pinned: bool,
    /// `true` ⇒ this process asserted the non-production opt-in that
    /// UNLOCKS the dev overrides below. A production KBS is `false`.
    pub vault_dev_environment: bool,
    /// `true` ⇒ the KBS-side measurement / TCB / launch-policy gates are
    /// WIDENED (dev only). Must be `false` in production.
    pub vault_dev_allow_any_kbs_measurement: bool,
    /// `true` ⇒ Vault server-cert verification is SKIPPED (dev only).
    /// Must be `false` in production.
    pub vault_dev_skip_tls_verify: bool,
}

/// 200 OK body for `POST /v1/admin/register-vm`. Echoes the fields
/// the KBS derived from the verified ticket so the caller can
/// confirm what was applied (no silent state drift).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRegisterVmResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// Echoed from the verified ticket.
    pub ticket_id: String,
    /// Echoed from the verified ticket.
    pub vm_id: String,
    /// Echoed from the verified ticket.
    pub vm_generation: u64,
    /// Echoed from the verified ticket (`platform_id` → `host`).
    pub host: String,
    /// Echoed from the verified ticket.
    pub lease_id: String,
    /// Unix seconds of the KBS-side apply.
    pub applied_at: u64,
    /// `true` ⇒ this exact `ticket_id` was previously applied with an
    /// identical body; KBS returned the cached response, no new
    /// state-store write. `false` ⇒ this was the first apply.
    pub cached: bool,
}

/// Request body for `POST /v1/admin/vm/{vm_id}/activate` (§25
/// destination re-activation). vali posts this as JSON (see
/// `vali/apps/orchestration/effects.py::kbs_activate_dest`). The KBS
/// uses ONLY `dest_node_id` + `new_gen` for the fence; it derives
/// `old_gen` / `source` / `lease_id` from its own authoritative
/// `Active{…}` state. `snapshot_get_url` is destined for the dest miner
/// (the disk download), not the KBS — it is accepted (so the body
/// round-trips) but never persisted or acted on here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminActivateRequest {
    /// The destination node id that will boot the migrated VM and attest
    /// at `new_gen`. Bound into the KBS `Migrating{dest}` so ONLY this
    /// node can unlock the rootfs KEK after the fence.
    pub dest_node_id: String,
    /// The forward-only generation the destination boots at. A stale
    /// generation can never unlock (`check_releasable` denies
    /// `ticket_gen != new_gen`) — replay / rollback protection.
    pub new_gen: u64,
    /// Short-TTL presigned S3 GET for the snapshot disk. For the dest
    /// MINER, not the KBS — optional + ignored here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_get_url: Option<String>,
}

/// 200 OK body for `POST /v1/admin/vm/{vm_id}/activate`. Echoes the
/// committed `Migrating` fields so vali can confirm the fence moved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminActivateResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// The VM that was re-activated for migration.
    pub vm_id: String,
    /// The fenced-out source generation (the KBS's prior `Active{gen}`).
    pub old_gen: u64,
    /// The destination generation that may now unlock.
    pub new_gen: u64,
    /// The destination node bound into `Migrating{dest}`.
    pub dest: String,
    /// `true` ⇒ the VM was already `Migrating` to this exact
    /// `(new_gen, dest)` — an idempotent re-drive, no fresh write.
    pub cached: bool,
}

/// 200 body for `POST /v1/admin/allowlist/reload`. Confirms what the
/// KBS swapped in so vali (the caller) can audit + log the new state.
///
/// The reload is the dev-mode automation for the §22 ceremony — vali
/// signs a fresh manifest with the dev seed, posts the COSE here,
/// and the KBS atomically swaps the in-memory body after the same
/// signature + epoch HWM checks the file-fed startup path runs. Same
/// `InstalledAllowlist::install` API powers both lanes; the only
/// difference is the transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminReloadAllowlistResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// Monotonic epoch from the newly-installed body. Vali pins this
    /// in its activity log so a later `register-vm` against a stale
    /// allowlist surfaces loudly.
    pub epoch: u64,
    /// Lower-case hex SHA-256 of the COSE bytes the KBS installed.
    /// Matches what the caller uploaded — a mismatch here means the
    /// KBS decoded a different byte sequence (the request body was
    /// mangled in transit) and the install would have already
    /// rejected on signature; included for belt-and-suspenders.
    pub sha256_hex: String,
}

/// 4xx body (400/401/403/409/429). Symmetric across error codes so
/// the client can deserialize without branching on status. The exact
/// status code carries the high-level class; `reason` is the
/// machine-grep subclass string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminErrorResponse {
    /// Short machine-grep subclass — e.g. `"ticket-decode"`,
    /// `"ticket-expired"`, `"url-vm-id-mismatch"`,
    /// `"state-divergent"`, `"unauthorized"`, `"forbidden"`,
    /// `"rate-limited"`. Stable across versions — the test suite
    /// pins each of them.
    pub reason: String,
    /// Optional ticket id, when the body parsed far enough for KBS
    /// to extract it. Absent on 400 `ticket-decode`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Optional vm id; same conditions as `ticket_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_op_path_segments_match_vali_effects_py() {
        // Pinned to match `vali/apps/orchestration/effects.py:_kbs_post`
        // path component naming. Renaming any of these is a
        // wire-breaking change and MUST land in lockstep on both sides.
        assert_eq!(AdminOp::RegisterVm.path_segment(), "register-vm");
        assert_eq!(AdminOp::Decommission.path_segment(), "decommission");
        assert_eq!(AdminOp::CryptoErase.path_segment(), "crypto-erase");
        assert_eq!(AdminOp::Activate.path_segment(), "activate");
        assert_eq!(AdminOp::SeedBootCounter.path_segment(), "seed-boot-counter");
        assert_eq!(
            AdminOp::ResetVolumeStampSuppression.path_segment(),
            "reset-volume-stamp-suppression"
        );
        assert_eq!(
            AdminOp::ArmBootCounterResync.path_segment(),
            "arm-boot-counter-resync"
        );
    }

    #[test]
    fn admin_arm_boot_counter_resync_response_round_trips() {
        // The shape carries no operator-chosen counter — the re-baseline
        // target is the KBS's own `stored + 1`. `stored` is echoed for
        // confirmation only.
        let r = AdminArmBootCounterResyncResponse {
            v: 1,
            vm_id: "vm-1".into(),
            stored: 4,
            already_armed: true,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminArmBootCounterResyncResponse =
            ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.vm_id, "vm-1");
        assert_eq!(back.stored, 4);
        assert!(back.already_armed);
    }

    #[test]
    fn admin_config_posture_response_round_trips() {
        // The two fields a monitor diffs first: the resolved
        // suppressed-confirm bound (`None` ⇒ DISABLED) and the admin
        // listener mode actually chosen. Both must survive the wire.
        let r = AdminConfigPostureResponse {
            v: 1,
            require_wrapped_kek: true,
            max_unconfirmed_releases: Some(3),
            volume_stamp_gate_armed: true,
            admin_listener_mode: "mtls".into(),
            evidence_sink_wired: true,
            live_attestation_sink_wired: true,
            allowlist_root_pubkey_fpr: "0123456789abcdef".into(),
            allowlist_root_next_pubkey_fpr: None,
            allowlist_signed_path_configured: true,
            l1_key_count: 1,
            min_tcb: 0,
            required_bits: 0,
            allowed_mask: 0,
            snp_chain_wired: true,
            snp_generation: Some("turin".into()),
            snp_kds_fetch_enabled: true,
            vault_broker_wired: true,
            vault_broker_ca_pinned: true,
            vault_ca_pinned: true,
            vault_dev_environment: false,
            vault_dev_allow_any_kbs_measurement: false,
            vault_dev_skip_tls_verify: false,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminConfigPostureResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.max_unconfirmed_releases, Some(3));
        assert_eq!(back.admin_listener_mode, "mtls");

        // `None` is the DISABLED state and must not decode as `Some(0)`
        // — a monitor that saw 0 would read "bound = 0" (refuse
        // everything) instead of "gate off".
        let disabled = AdminConfigPostureResponse {
            max_unconfirmed_releases: None,
            volume_stamp_gate_armed: false,
            ..r
        };
        let mut dbytes = Vec::new();
        ciborium::ser::into_writer(&disabled, &mut dbytes).unwrap();
        let dback: AdminConfigPostureResponse =
            ciborium::de::from_reader(dbytes.as_slice()).unwrap();
        assert!(dback.max_unconfirmed_releases.is_none());
        assert!(!dback.volume_stamp_gate_armed);
    }

    #[test]
    fn admin_reset_volume_stamp_suppression_response_round_trips() {
        let r = AdminResetVolumeStampSuppressionResponse {
            v: 1,
            vm_id: "vm-1".into(),
            cleared: 4,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminResetVolumeStampSuppressionResponse =
            ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.vm_id, "vm-1");
        assert_eq!(back.cleared, 4);
    }

    #[test]
    fn admin_seed_boot_counter_round_trips() {
        let r = AdminSeedBootCounterRequest { counter: 12 };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminSeedBootCounterRequest =
            ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.counter, 12);

        let resp = AdminSeedBootCounterResponse {
            v: 1,
            vm_id: "vm-1".into(),
            previous: 0,
            counter: 12,
        };
        let mut rbytes = Vec::new();
        ciborium::ser::into_writer(&resp, &mut rbytes).unwrap();
        let rback: AdminSeedBootCounterResponse =
            ciborium::de::from_reader(rbytes.as_slice()).unwrap();
        assert_eq!(rback.previous, 0);
        assert_eq!(rback.counter, 12);
    }

    #[test]
    fn admin_op_serde_kebab_case() {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&AdminOp::RegisterVm, &mut buf).unwrap();
        // `serde(rename_all = "kebab-case")` encodes the variant as a
        // CBOR text string. Just round-trip and check the symbolic
        // identity — the exact CBOR bytes are an implementation detail
        // of ciborium's enum encoder.
        let back: AdminOp = ciborium::de::from_reader(buf.as_slice()).unwrap();
        assert_eq!(back, AdminOp::RegisterVm);
    }

    #[test]
    fn admin_register_vm_response_round_trip() {
        let r = AdminRegisterVmResponse {
            v: 1,
            ticket_id: "tk-1".into(),
            vm_id: "vm-1".into(),
            vm_generation: 3,
            host: "chip-aa84a3f0".into(),
            lease_id: "lease-q2".into(),
            applied_at: 1700000000,
            cached: false,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminRegisterVmResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.ticket_id, "tk-1");
        assert_eq!(back.vm_generation, 3);
        assert!(!back.cached);
    }

    #[test]
    fn admin_activate_request_round_trips_and_url_is_optional() {
        let r = AdminActivateRequest {
            dest_node_id: "dst-node".into(),
            new_gen: 6,
            snapshot_get_url: Some("https://s3/get?sig=x".into()),
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminActivateRequest = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.dest_node_id, "dst-node");
        assert_eq!(back.new_gen, 6);
        assert_eq!(
            back.snapshot_get_url.as_deref(),
            Some("https://s3/get?sig=x")
        );

        // The snapshot URL is optional (`serde(default)`) — a body
        // without it still decodes (vali may omit it).
        let minimal = AdminActivateRequest {
            dest_node_id: "d".into(),
            new_gen: 2,
            snapshot_get_url: None,
        };
        let mut mbytes = Vec::new();
        ciborium::ser::into_writer(&minimal, &mut mbytes).unwrap();
        let mback: AdminActivateRequest = ciborium::de::from_reader(mbytes.as_slice()).unwrap();
        assert_eq!(mback.new_gen, 2);
        assert!(mback.snapshot_get_url.is_none());
    }

    #[test]
    fn admin_activate_response_round_trip() {
        let r = AdminActivateResponse {
            v: 1,
            vm_id: "vm-1".into(),
            old_gen: 5,
            new_gen: 6,
            dest: "dst".into(),
            cached: false,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminActivateResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.old_gen, 5);
        assert_eq!(back.new_gen, 6);
        assert!(!back.cached);
    }

    #[test]
    fn admin_error_response_round_trip() {
        let e = AdminErrorResponse {
            reason: "state-divergent".into(),
            ticket_id: Some("tk-2".into()),
            vm_id: Some("vm-2".into()),
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&e, &mut bytes).unwrap();
        let back: AdminErrorResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.reason, "state-divergent");
        assert_eq!(back.ticket_id.as_deref(), Some("tk-2"));
    }
}
