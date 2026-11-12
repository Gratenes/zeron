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
    fn call(&self, tool: String, args_json: String) -> PhoneToolReply;
}

struct Bridge(Arc<dyn PhoneToolHandler>);

impl zc::PhoneTools for Bridge {
    fn call(&self, tool: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        let reply = self.0.call(tool.to_owned(), args.to_string());
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
    /// tools with `handler`. Safe to call again (replaces the host).
    pub fn start_phone_host(&self, handler: Arc<dyn PhoneToolHandler>) -> CoreResult<()> {
        self.client.start_phone_host(Arc::new(Bridge(handler))).map_err(Into::into)
    }

    pub fn stop_phone_host(&self) {
        self.client.stop_phone_host();
    }
}
