//! Tektronix scope: VISA transport, SCPI client, CURVe / HARDCopy.

pub(crate) mod binary;
mod client;
pub(crate) mod visa;

pub use client::{
    capture_shot, default_save_dir, list_scopes, read_waveform_json, scope_capabilities,
    ScopeError, ScopeInfo, TektronixScopeClient, Waveform, TEK_VID,
};

/// CLI-compatible waveform entry (returns JSON).
pub fn read_waveform(
    index: usize,
    channel: &str,
    points: Option<u32>,
) -> Result<serde_json::Value, ScopeError> {
    read_waveform_json(index, channel, points)
}
