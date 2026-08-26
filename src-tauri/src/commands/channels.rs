//! Tauri IPC commands for the Phase 6 channels feature (Discord / Telegram).
//!
//! Request bodies are camelCase at the IPC boundary (`channel`, `dmPolicy`,
//! `allowFrom`, `groupPolicy`, `code`); errors are the unified `AppError`
//! (stable code + masked message). The channel token travels to Rust only —
//! it never appears in argv, config, logs, or the UI (S3/S7/S8).

use crate::application::{ChannelService, ChannelTokenService};
use crate::domain::models::channels::{ChannelConfig, ChannelsOverview, PairingRequest};
use crate::error::AppError;

/// Runs a blocking task on Tauri's blocking thread pool (CLI calls) so the
/// UI thread is never blocked (`plugins install` may take minutes).
async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("process-failed", "command task failed unexpectedly"))?
}

/// `get-channels`: merged `channels list --all` + `channels status` rows for
/// discord/telegram (read-only; absent rows fail soft to false).
#[tauri::command(rename = "get-channels")]
pub async fn get_channels() -> Result<ChannelsOverview, AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.get_channels()).await
}

/// `get-channel-config`: redacted `channels.<channel>` snapshot
/// (fail-soft parse; token is a managed/external/absent state only).
#[tauri::command(rename = "get-channel-config")]
pub async fn get_channel_config(channel: String) -> Result<ChannelConfig, AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.get_channel_config(&channel)).await
}

/// `set-channel-token`: DPAPI-first registration of the channel token.
/// The token value is validated (non-secret error on failure) and stored in
/// the OS secret store; only the exec SecretRef lands in the config.
#[tauri::command(rename = "set-channel-token")]
pub async fn set_channel_token(channel: String, token: String) -> Result<(), AppError> {
    let service = ChannelTokenService::production();
    run_blocking(move || service.set_channel_token(&channel, &token)).await
}

/// `delete-channel-token`: removes the managed ref + DPAPI entry (orphan
/// cleanup supported) and the shared provider declaration when no surface
/// holds a managed ref anymore.
#[tauri::command(rename = "delete-channel-token")]
pub async fn delete_channel_token(channel: String) -> Result<(), AppError> {
    let service = ChannelTokenService::production();
    run_blocking(move || service.delete_channel_token(&channel)).await
}

/// `connect-channel`: token-ref precondition → (Discord) idempotent plugin
/// install → `enabled=true`. Fixed order, first failure stops.
#[tauri::command(rename = "connect-channel")]
pub async fn connect_channel(channel: String) -> Result<(), AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.connect_channel(&channel)).await
}

/// `set-channel-enabled`: scalar `enabled` write (disable keeps token and
/// policies).
#[tauri::command(rename = "set-channel-enabled")]
pub async fn set_channel_enabled(channel: String, enabled: bool) -> Result<(), AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.set_channel_enabled(&channel, enabled)).await
}

/// `set-dm-access`: `dmPolicy` → `allowFrom` (`--replace`) in a fixed order.
/// Pre-validated (S2: 0 process runs on an invalid combination).
#[tauri::command(rename = "set-dm-access")]
pub async fn set_dm_access(
    channel: String,
    dm_policy: String,
    allow_from: Vec<String>,
) -> Result<(), AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.set_dm_access(&channel, &dm_policy, &allow_from)).await
}

/// `set-group-policy`: enum-validated scalar `groupPolicy` write.
#[tauri::command(rename = "set-group-policy")]
pub async fn set_group_policy(channel: String, group_policy: String) -> Result<(), AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.set_group_policy(&channel, &group_policy)).await
}

/// `list-pairing-requests`: pending pairing requests for the channel.
#[tauri::command(rename = "list-pairing-requests")]
pub async fn list_pairing_requests(channel: String) -> Result<Vec<PairingRequest>, AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.list_pairing_requests(&channel)).await
}

/// `approve-pairing`: channel + code validated before the CLI call (S2).
#[tauri::command(rename = "approve-pairing")]
pub async fn approve_pairing(channel: String, code: String) -> Result<(), AppError> {
    let service = ChannelService::production();
    run_blocking(move || service.approve_pairing(&channel, &code)).await
}
