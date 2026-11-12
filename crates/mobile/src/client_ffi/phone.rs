//! Agents' phone tools: the platform's [`PhoneToolHandler`] behind the
//! client's device-room host (`zeron_client::phone_host`).

use std::sync::Arc;

use zeron_client as zc;

use super::{CoreClient, CoreResult};

/// What a tool call answers: `json` is the result when `ok`, else the message
/// the agent reads.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PhoneToolReply {
    pub ok: bool,
    pub json: String,
}

/// The platform's phone tools. Called on a background thread, so a tool may
/// block (the screen read, a confirmation the person answers).
#[uniffi::export(with_foreign)]
pub trait PhoneToolHandler: Send + Sync {
    /// `origin_json` is `{chatId, chatTitle}` of the asking agent's chat, or empty.
    fn call(&self, tool: String, args_json: String, origin_json: String) -> PhoneToolReply;
}

struct Bridge(Arc<dyn PhoneToolHandler>);

impl zc::PhoneTools for Bridge {
    fn call(
        &self,
        tool: &str,
        args: serde_json::Value,
        origin: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let reply = self.0.call(
            tool.to_owned(),
            args.to_string(),
            origin.map(|o| o.to_string()).unwrap_or_default(),
        );
        if reply.ok {
            serde_json::from_str(&reply.json).map_err(|e| format!("the phone's {tool} reply wasn't JSON: {e}"))
        } else {
            Err(reply.json)
        }
    }
}

#[uniffi::export]
impl CoreClient {
    /// Join this phone's own device room as its host and serve agents' phone
    /// tools with `handler`, pinging the room every `keepalive_secs` (10-300; each ping
    /// wakes the radio). Safe to call again (replaces the host).
    pub fn start_phone_host(&self, handler: Arc<dyn PhoneToolHandler>, keepalive_secs: u32) -> CoreResult<()> {
        let keepalive = std::time::Duration::from_secs(u64::from(keepalive_secs.clamp(10, 300)));
        self.client.start_phone_host(Arc::new(Bridge(handler)), keepalive).map_err(Into::into)
    }

    pub fn stop_phone_host(&self) {
        self.client.stop_phone_host();
    }
}
