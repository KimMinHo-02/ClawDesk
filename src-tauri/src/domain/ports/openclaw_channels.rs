//! OpenClaw channels port (Phase 6).
//!
//! Every method maps to a non-interactive `openclaw channels` /
//! `openclaw pairing` CLI invocation (structured argv via `ProcessPort` —
//! S1/S2). Channel ids and pairing codes are validated before use and
//! passed as single argv elements. No live probe / credentials / lifecycle
//! commands (non-goals).

use std::path::Path;

use crate::domain::models::channels::{ChannelRow, ChannelStatus, PairingRequest};
use crate::error::AppError;

pub trait OpenClawChannelsPort: Send + Sync {
    /// `openclaw channels list --all --json` → configured account rows
    /// (read-only, 30s).
    fn list_channels(&self, executable: &Path) -> Result<Vec<ChannelRow>, AppError>;

    /// `openclaw channels status --json` → gateway reachability + per-channel
    /// runtime state (read-only, 30s). Gateway-unreachable falls back to the
    /// config-only summary (`gateway_reachable: false`).
    fn channel_status(&self, executable: &Path) -> Result<ChannelStatus, AppError>;

    /// `openclaw pairing list <channel> --json` → pending pairing requests
    /// (read-only, 30s).
    fn pairing_list(
        &self,
        executable: &Path,
        channel: &str,
    ) -> Result<Vec<PairingRequest>, AppError>;

    /// `openclaw pairing approve <channel> <code>` → approves one pending
    /// code (30s). The code is a single argv element (S2). The first
    /// approval may bootstrap `commands.ownerAllowFrom` (one-time owner
    /// bootstrap — no extra CLI call is made).
    fn pairing_approve(&self, executable: &Path, channel: &str, code: &str)
        -> Result<(), AppError>;
}
