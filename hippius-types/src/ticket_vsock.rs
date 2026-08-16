//! Constants for the host→guest AF_VSOCK OrderTicket delivery channel.
//!
//! The OrderTicket COSE_Sign1 envelope **must not** ride a measured
//! surface (kernel cmdline or QEMU `-fw_cfg` are both folded into the
//! §22 launch_digest; per-launch ticket data there would explode the
//! allowlist — one entry per launch). This module declares the
//! constants both the producer (miner-agent, `binaries/miner-agent/`)
//! and the consumer (initramfs agent, `binaries/agent-initramfs/`)
//! agree on for the runtime push channel.
//!
//! ## Channel shape
//!
//! - **Direction.** Host → guest. The miner-agent (always AF_VSOCK
//!   CID `2` by ABI) connects to the tenant guest's assigned CID on
//!   [`PORT`]. The guest listens on `(VMADDR_CID_ANY, PORT)`.
//! - **Boot race.** The host retries `connect(2)` for up to
//!   [`PUSH_TIMEOUT_SECS`] so a guest whose initramfs has not yet
//!   reached the listen step is not racing the dispatch.
//! - **Wire format.** A single message per connection:
//!   `u32` big-endian length prefix + that many raw COSE bytes.
//!   The big-endian choice matches the existing miner-agent ↔ guest
//!   relay framing (`binaries/miner-agent/src/vsock/frame.rs`). The
//!   length is capped at [`MAX_TICKET_BYTES`] before any allocation.
//! - **One-shot.** The host closes after writing; the guest closes
//!   after reading. There is no kind discriminator — the port itself
//!   identifies the channel.

/// AF_VSOCK port on which the guest agent-initramfs listens for the
/// signed OrderTicket. `0x4849` = "HI" (Hippius). Stable across the
/// Phase A protocol — a future protocol revision uses a new port
/// rather than re-purposing this one (avoids a measured-image / host
/// version-skew foot-gun).
pub const PORT: u32 = 0x4849;

/// Cap on one ticket frame's COSE body. A COSE_Sign1 OrderTicket is a
/// few hundred bytes; 8 KiB is generous head-room while keeping a
/// hostile-length rejection a bounded operation. Mirrors the
/// per-channel cap discipline of the relay's `MAX_VSOCK_FRAME`.
pub const MAX_TICKET_BYTES: usize = 8 * 1024;

/// How long the host retries `connect(2)` on the ticket port before
/// failing the launch. The kernel boot → initramfs `/init` → vsock
/// listen prep typically takes 1–3 seconds; 30 s absorbs a slow boot
/// without indefinitely tying up the miner-agent dispatch task.
pub const PUSH_TIMEOUT_SECS: u64 = 180;

/// How long the guest waits on an inbound ticket connection before
/// failing closed. Symmetrically bounds the host-side timeout so a
/// host that never connects does not hang the initramfs.
pub const ACCEPT_TIMEOUT_SECS: u64 = 180;
