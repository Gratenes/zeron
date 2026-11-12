//! `PhoneCall {targetDeviceId, tool, args}`: an agent's tool call, forwarded to
//! the account's phone (Zeron for Android, which hosts its own device room) and
//! answered by the phone. This engine never answers one itself.
//!
//! Only a device in this account's list is asked, the phone's own per-tool
//! switches decide what it does, and the ask is bounded by [`DEADLINE`].

use std::time::Duration;

use chrono::Utc;
use zeron_rpc::{RpcError, RpcReply, methods};

use super::EngineRpc;
use super::device_stats::{is_away, remote_error};

/// The most a phone may take, dial included. Longer than a stats report: a
/// screenshot or a send waits on a person's confirmation.
const DEADLINE: Duration = Duration::from_secs(45);

/// How long a phone that is away is waited for before saying so. A phone that
/// keeps no socket open is woken by a push when the call arrives and dials its
/// host in a few seconds; this is the wait for that dial.
const WAKE_WAIT: Duration = Duration::from_secs(20);
const WAKE_POLL: Duration = Duration::from_millis(500);

impl EngineRpc {
    pub(super) async fn phone_call(&self, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        let target = params
            .get("targetDeviceId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| RpcError::BadParams("PhoneCall needs targetDeviceId".into()))?;
        if target == self.doc_host.device_id() {
            return Err(RpcError::Failed("This device isn't a phone".into()));
        }
        let devices = self.workspace.watch_devices().borrow().clone();
        let Some(device) = devices.iter().find(|d| d.id == target) else {
            return Err(RpcError::BadParams(format!(
                "{target} isn't one of this account's devices"
            )));
        };
        if self.links.is_none() {
            return Err(RpcError::Failed(format!(
                "Can't reach {}: this device isn't connected (signed out or offline)",
                device.name
            )));
        }
        let started = tokio::time::Instant::now();
        let asked = loop {
            let left = DEADLINE.saturating_sub(started.elapsed());
            let reply = tokio::time::timeout(left, self.forward(target, methods::PHONE_CALL, params.clone())).await;
            match reply {
                Ok(Err(error)) if is_away(&error) && started.elapsed() < WAKE_WAIT => {
                    tracing::debug!(%target, %error, "phone call: the phone is away, waiting for it");
                    if let Some(links) = &self.links {
                        links.invalidate(target);
                    }
                    tokio::time::sleep(WAKE_POLL).await;
                }
                other => break other,
            }
        };
        match asked {
            Ok(Ok(reply @ RpcReply::Value(_))) => Ok(reply),
            Ok(Ok(RpcReply::Stream(_))) => Err(RpcError::Failed(format!(
                "{} answered PhoneCall with a stream",
                device.name
            ))),
            Ok(Err(RpcError::UnknownMethod(_))) => Err(RpcError::Failed(format!(
                "{} doesn't run Zeron's phone tools (update Zeron for Android there)",
                device.name
            ))),
            Ok(Err(error)) => Err(remote_error(device, error, Utc::now())),
            Err(_) => {
                if let Some(links) = &self.links {
                    links.invalidate(target);
                }
                Err(RpcError::Failed(format!(
                    "{} didn't answer within {} s: the phone may be asleep, offline, or waiting on a person",
                    device.name,
                    DEADLINE.as_secs()
                )))
            }
        }
    }
}
