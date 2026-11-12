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

/// A link kept open to the phone pings it every 10 s (the client link's keepalive and echo),
/// each ping waking its radio. So the link is let go [`LINK_IDLE`] after the last call, and
/// the next call dials again (a second or two, and the phone is awake for it anyway).
const LINK_IDLE: Duration = Duration::from_secs(30);

/// The latest call to each phone, so only the last one's idle timer closes the link.
static LAST_CALL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
    std::sync::LazyLock::new(Default::default);

/// Whether a call to `device` has armed the timer that lets its link go (for tests).
pub(crate) fn link_idle_pending(device: &str) -> bool {
    LAST_CALL.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains_key(device)
}

/// A phone that restarted keeps its agent link down until the person has unlocked it once by hand.
fn with_restart_hint(error: RpcError) -> RpcError {
    match error {
        RpcError::Failed(message) if message.contains("is offline") => RpcError::Failed(format!(
            "{message}. If the phone restarted, its agent link comes back after the person unlocks it once by hand."
        )),
        other => other,
    }
}

impl EngineRpc {
    /// One ask of the phone over its device room. A phone is dialed whatever its registry
    /// presence says (it rests its workspace sockets): the room tells at once if its host is away.
    async fn ask_phone(&self, target: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        let links = self.links.as_ref().ok_or_else(|| RpcError::Failed("peer links unavailable (offline)".into()))?;
        links.reset_cooldown(target);
        let client = links.client_ignoring_presence(target).await?;
        match client.call(methods::PHONE_CALL, params).await {
            Ok(value) => Ok(RpcReply::Value(value)),
            Err(error) => {
                if matches!(error, RpcError::Closed) {
                    links.invalidate(target);
                }
                Err(error)
            }
        }
    }

    /// Close the link to `target` after [`LINK_IDLE`] with no newer call.
    fn close_phone_link_when_idle(&self, target: &str) {
        let Some(links) = self.links.clone() else { return };
        let generation = {
            let mut last = LAST_CALL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let next = last.get(target).copied().unwrap_or(0) + 1;
            last.insert(target.to_owned(), next);
            next
        };
        let target = target.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(LINK_IDLE).await;
            let mut last = LAST_CALL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if last.get(&target) == Some(&generation) {
                last.remove(&target);
                links.invalidate(&target);
            }
        });
    }

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
            let reply = tokio::time::timeout(left, self.ask_phone(target, params.clone())).await;
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
        self.close_phone_link_when_idle(target);
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
            Ok(Err(error)) => Err(with_restart_hint(remote_error(device, error, Utc::now()))),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offline_phone_is_told_to_be_unlocked_by_hand_if_it_restarted() {
        let said = |e: RpcError| match e {
            RpcError::Failed(m) => m,
            other => panic!("{other}"),
        };
        let away = with_restart_hint(RpcError::Failed("Pixel is offline (last seen 3 h ago)".into()));
        assert!(said(away).contains("unlocks it once by hand"));
        // Anything else (the phone's own refusal) passes through untouched.
        let refusal = with_restart_hint(RpcError::Failed("Pixel: Reading the screen is switched off".into()));
        assert_eq!(said(refusal), "Pixel: Reading the screen is switched off");
    }
}
