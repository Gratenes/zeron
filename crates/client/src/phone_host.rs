//! The phone as a device-room host for agents' `phone_*` tools.
//!
//! The phone already joins its account's rooms as a client. This also joins
//! its OWN device room as the host (the same [`zeron_rpc::HostRelay`] the
//! desktop engine runs), serving exactly one method, `PhoneCall
//! {tool, args}`, to the account's other devices. An agent's engine forwards
//! the call there (`PhoneCall` in the engine's rpc); everything else the relay
//! is asked is "unknown method". No engine runs on the phone.
//!
//! What a tool does, whether it is switched on and whether the person has to
//! confirm it are the platform's (`PhoneTools`); this crate only carries the
//! call.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zeron_rpc::{RpcError, RpcReply, RpcService, methods};

use crate::client::Client;
use crate::error::{ClientError, Result};

/// The platform's tools. Called on a blocking thread, one call at a time per
/// request: a tool may take seconds (waiting on the person's confirmation).
/// `Err` is a message for the agent, not a failure of the link.
pub trait PhoneTools: Send + Sync + 'static {
    /// `origin` is who asked, as the agent's engine says it (`{chatId, chatTitle}`), for the
    /// person's notice and log: never a reason to trust the call more.
    fn call(&self, tool: &str, args: Value, origin: Option<Value>) -> std::result::Result<Value, String>;
}

/// `Device::capabilities` of a phone that answers `PhoneCall`.
pub const PHONE_TOOLS_CAPABILITY: &str = "phone-tools-v1";

struct PhoneService(Arc<dyn PhoneTools>);

#[async_trait]
impl RpcService for PhoneService {
    async fn handle(&self, method: &str, params: Value) -> std::result::Result<RpcReply, RpcError> {
        // The dial's own checks (zeron_rpc::is_liveness_probe: a peer proves the host answers
        // before it keeps the link): a phone has no agents to run, and is ready.
        if method == methods::LIST_HARNESSES {
            return Ok(RpcReply::Value(json!([])));
        }
        if method == methods::ENGINE_READY {
            return Ok(RpcReply::Value(json!({ "ready": true })));
        }
        if method != methods::PHONE_CALL {
            return Err(RpcError::UnknownMethod(method.into()));
        }
        let tool = params
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::BadParams("PhoneCall needs tool".into()))?
            .to_owned();
        let args = params.get("args").cloned().unwrap_or_else(|| json!({}));
        let origin = params.get("origin").filter(|o| !o.is_null()).cloned();
        let tools = self.0.clone();
        let result = tokio::task::spawn_blocking(move || tools.call(&tool, args, origin))
            .await
            .map_err(|e| RpcError::Failed(format!("phone tool panicked: {e}")))?;
        match result {
            Ok(value) => Ok(RpcReply::Value(value)),
            Err(message) => Err(RpcError::Failed(message)),
        }
    }
}

impl Client {
    /// Host this phone's device room and serve `PhoneCall` with `tools`.
    /// Replaces a running host; [`Client::stop_phone_host`] and `shutdown`
    /// end it. Reconnects on its own (the relay's backoff and network wakes).
    pub fn start_phone_host(&self, tools: Arc<dyn PhoneTools>, keepalive: Duration) -> Result<()> {
        if self.inner.is_demo() {
            return Err(ClientError::Unsupported("phone tools need a signed-in account".into()));
        }
        // Agents find the phone as a device (`list_devices`, `phone_*`'s default
        // target): its row says it serves phone tools, and the engine reads
        // that capability off the row before it asks.
        let device = zeron_proto::Device {
            id: self.inner.config.device_id.clone(),
            name: self.inner.config.device_name.clone(),
            platform: self.inner.config.platform.clone(),
            last_seen_at: Some(chrono::Utc::now()),
            created_at: None,
            version: Some(self.inner.config.app_version.clone()),
            cursor_sdk_version: None,
            capabilities: vec![PHONE_TOOLS_CAPABILITY.to_owned()],
        };
        self.inner.registry_write(|doc| doc.upsert_device(&device))?;
        let mut config = zeron_rpc::HostRelayConfig::new(
            self.inner.config.edge_base().to_owned(),
            self.inner.config.device_id.clone(),
            Arc::new(crate::live::Bearer(Arc::downgrade(&self.inner))),
        );
        // A phone pings seldom: every ping wakes the radio. 60 s is the usual choice.
        config.keepalive = Some(keepalive);
        let _guard = crate::runtime::shared().enter();
        let relay = zeron_rpc::HostRelay::spawn(config, Arc::new(PhoneService(tools)), Arc::new(|_| true));
        *self.inner.phone_host.lock().unwrap_or_else(|e| e.into_inner()) = Some(relay);
        Ok(())
    }

    pub fn stop_phone_host(&self) {
        *self.inner.phone_host.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records what the platform was asked; `fail` answers every call with a refusal.
    struct Platform {
        asked: Mutex<Vec<(String, Value, Option<Value>)>>,
        fail: bool,
    }

    impl PhoneTools for Platform {
        fn call(&self, tool: &str, args: Value, origin: Option<Value>) -> std::result::Result<Value, String> {
            self.asked.lock().unwrap().push((tool.into(), args.clone(), origin));
            if self.fail { Err("Reading the screen is switched off on this phone".into()) } else { Ok(json!({ "echo": args })) }
        }
    }

    fn service(fail: bool) -> (PhoneService, Arc<Platform>) {
        let platform = Arc::new(Platform { asked: Mutex::new(Vec::new()), fail });
        (PhoneService(platform.clone()), platform)
    }

    #[tokio::test]
    async fn the_dials_own_probes_are_answered_without_asking_the_platform() {
        let (service, platform) = service(false);
        let harnesses = service.handle(methods::LIST_HARNESSES, json!({ "readinessProbe": true })).await.unwrap();
        assert!(matches!(harnesses, RpcReply::Value(v) if v == json!([])), "a phone runs no agents");
        let ready = service.handle(methods::ENGINE_READY, json!({})).await.unwrap();
        assert!(matches!(ready, RpcReply::Value(v) if v == json!({ "ready": true })));
        assert!(platform.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_phone_call_runs_the_platform_tool_with_its_args_and_origin() {
        let (service, platform) = service(false);
        let reply = service
            .handle(methods::PHONE_CALL, json!({ "tool": "tap", "args": { "node": "n1" }, "origin": { "chatId": "c" }, "targetDeviceId": "me" }))
            .await
            .unwrap();
        assert!(matches!(reply, RpcReply::Value(v) if v == json!({ "echo": { "node": "n1" } })));
        let asked = platform.asked.lock().unwrap().clone();
        assert_eq!(asked, vec![("tap".to_string(), json!({ "node": "n1" }), Some(json!({ "chatId": "c" })))]);

        // No args is an empty object, a null origin is none.
        service.handle(methods::PHONE_CALL, json!({ "tool": "ping", "origin": null })).await.unwrap();
        assert_eq!(platform.asked.lock().unwrap()[1], ("ping".to_string(), json!({}), None));
    }

    #[tokio::test]
    async fn a_refusal_reaches_the_agent_in_the_phones_words() {
        let (service, _) = service(true);
        let error = service.handle(methods::PHONE_CALL, json!({ "tool": "screen_read" })).await.err().unwrap();
        assert!(matches!(&error, RpcError::Failed(m) if m.contains("switched off")), "{error}");
    }

    #[tokio::test]
    async fn anything_else_is_unknown_and_a_call_needs_a_tool() {
        let (service, platform) = service(false);
        assert!(matches!(service.handle("ListChats", json!({})).await, Err(RpcError::UnknownMethod(_))));
        assert!(matches!(service.handle(methods::PHONE_CALL, json!({ "args": {} })).await, Err(RpcError::BadParams(_))));
        assert!(platform.asked.lock().unwrap().is_empty());
    }
}
