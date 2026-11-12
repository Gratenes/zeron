//! Phone tools: the account's Android phone (Zeron for Android) answers these
//! itself, over its own device-room host. The engine only routes (`PhoneCall`);
//! what each tool does, whether it is switched on and whether the person must
//! confirm are all the phone's to decide, and a refusal comes back as text.

use serde_json::{Value, json};
use zeron_rpc::methods;

use super::{ToolDef, Tools};

pub(super) const PHONE_PING: &str = "phone_ping";
pub(super) const PHONE_SCREEN_READ: &str = "phone_screen_read";

pub(super) const NAMES: [&str; 2] = [PHONE_PING, PHONE_SCREEN_READ];

fn device_arg() -> Value {
    json!({ "type": "string", "description": "The phone's device id or exact name. Omit when the account has exactly one Android phone." })
}

pub(super) fn catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: PHONE_PING,
            description: "Check that the person's Android phone is reachable and what it allows agents to do: its name, Android version, battery, whether Zeron's accessibility service is on, which tool groups the person has switched on in the app, and the foreground app. Call this first; every other phone_* tool fails with a plain reason when the phone is away or the tool is off.",
            input_schema: json!({ "type": "object", "properties": { "device": device_arg() } }),
        },
        ToolDef {
            name: PHONE_SCREEN_READ,
            description: "Read what the phone's screen shows now, as the accessibility tree of the foreground window: the app's package, then each visible node with an id, class, text, content description, bounds (left, top, right, bottom in pixels) and flags (clickable, editable, scrollable, checked, focused). Password fields read as redacted. The person sees an 'agent is using this phone' indicator while you work.",
            input_schema: json!({ "type": "object", "properties": {
                "device": device_arg(),
                "max_nodes": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Cap on nodes returned. Default 400." }
            } }),
        },
    ]
}

impl Tools {
    pub(super) async fn phone(&self, tool: &str, mut args: Value) -> Result<Value, String> {
        if args.is_null() {
            args = json!({});
        }
        let Some(object) = args.as_object_mut() else {
            return Err("invalid arguments: expected an object".into());
        };
        let device = object.remove("device").and_then(|d| d.as_str().map(str::to_owned));
        let target = self.phone_device(device.as_deref()).await.map_err(|e| format!("{e:#}"))?;
        let name = tool.strip_prefix("phone_").unwrap_or(tool);
        self.zeron
            .call_rpc(
                methods::PHONE_CALL,
                json!({ "targetDeviceId": target, "tool": name, "args": args }),
            )
            .await
            .map_err(|e| e.to_string())
    }

    /// The phone to ask: the one named, else the account's only Android device.
    async fn phone_device(&self, key: Option<&str>) -> anyhow::Result<String> {
        if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) {
            return self.zeron.resolve_device_id(Some(key)).await;
        }
        let devices = self.zeron.devices().await?;
        let phones: Vec<_> = devices.iter().filter(|d| d.platform.eq_ignore_ascii_case("android")).collect();
        match phones.as_slice() {
            [one] => Ok(one.id.clone()),
            [] => anyhow::bail!("this account has no Android phone running Zeron (list_devices shows its devices)"),
            many => anyhow::bail!(
                "several Android phones: pass device as one of {}",
                many.iter().map(|d| format!("{} ({})", d.name, d.id)).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}
