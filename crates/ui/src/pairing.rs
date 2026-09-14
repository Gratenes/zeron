//! UI-side models for the IPC-only Tailcat pairing surface.
//!
//! Invitation contents stay opaque here: the engine validates and redeems the
//! signed `kratos-pair:` payload. The UI only trims pasted text and never
//! persists or logs it.

use serde::Deserialize;

pub const PAIRING_PREFIX: &str = "kratos-pair:";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    pub signed_in: bool,
    pub hosting: bool,
    pub connected: bool,
    pub profile_id: Option<String>,
    pub device_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedDevice {
    pub device_id: String,
    pub display_name: Option<String>,
    pub owner: bool,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

pub fn parse_peer_status(value: serde_json::Value) -> Result<PeerStatus, String> {
    serde_json::from_value(value).map_err(|_| "The engine returned an invalid peer status".into())
}

pub fn parse_trusted_devices(value: serde_json::Value) -> Result<Vec<TrustedDevice>, String> {
    serde_json::from_value(value.get("devices").cloned().unwrap_or_default())
        .map_err(|_| "The engine returned an invalid trusted-device list".into())
}

/// The engine owns decoding, signature checks, expiry, and redemption. This is
/// deliberately only a cheap empty/wrong-kind check for immediate field copy.
pub fn pairing_code_for_rpc(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        Err("Paste an invitation from a trusted device")
    } else if !value.starts_with(PAIRING_PREFIX) {
        Err("Invitation codes start with kratos-pair:")
    } else {
        Ok(value.to_owned())
    }
}

pub fn connectivity_label(status: &PeerStatus) -> &'static str {
    if !status.signed_in {
        "Not paired"
    } else if status.connected {
        "Connected"
    } else if status.last_error.as_deref().is_some_and(|error| {
        let error = error.to_ascii_lowercase();
        error.contains("revoked") || error.contains("unauthorized") || error.contains("forbidden")
    }) {
        "Access revoked"
    } else {
        "Peer offline"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitation_is_opaque_but_must_have_the_wire_prefix() {
        assert_eq!(
            pairing_code_for_rpc("  kratos-pair:secret  ").unwrap(),
            "kratos-pair:secret"
        );
        assert!(pairing_code_for_rpc("").is_err());
        assert!(pairing_code_for_rpc("https://example.test").is_err());
    }

    #[test]
    fn offline_is_not_reported_as_revocation_without_positive_evidence() {
        let mut status = PeerStatus {
            signed_in: true,
            ..PeerStatus::default()
        };
        assert_eq!(connectivity_label(&status), "Peer offline");
        status.last_error = Some("peer request timed out".into());
        assert_eq!(connectivity_label(&status), "Peer offline");
        status.last_error = Some("device is revoked".into());
        assert_eq!(connectivity_label(&status), "Access revoked");
    }

    #[test]
    fn parses_peer_device_contract() {
        let rows = parse_trusted_devices(serde_json::json!({"devices": [{
            "deviceId": "device-1", "displayName": "Laptop", "owner": true,
            "createdAt": 7, "revokedAt": null
        }]}))
        .unwrap();
        assert_eq!(rows[0].device_id, "device-1");
        assert!(rows[0].owner);
    }
}
