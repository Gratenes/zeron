//! Device file tools: look at, and copy, files between the account's devices
//! (this machine, another computer, a phone) without anyone driving a screen.
//! The engine does the moving (`DeviceFileCopy`, chunked over the device relay,
//! resumable, checked by sha256); each device decides what it exposes.

use std::time::Duration;

use serde_json::{Value, json};
use zeron_rpc::{RpcError, methods};

use super::{ToolDef, Tools};

pub(super) const LIST: &str = "device_files_list";
pub(super) const STAT: &str = "device_file_stat";
pub(super) const COPY: &str = "device_file_copy";
pub(super) const COPY_STATUS: &str = "device_file_copy_status";

pub(super) const NAMES: [&str; 4] = [LIST, STAT, COPY, COPY_STATUS];

/// How long a copy call waits for its transfer before answering `running`.
const WAIT: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_millis(500);

pub(super) fn catalog() -> Vec<ToolDef> {
    let device = json!({ "type": "string", "description": "Device id or exact name (list_devices). Omit for this machine." });
    vec![
        ToolDef {
            name: LIST,
            description: "List a folder on one of the account's devices: name, full path, kind (file or dir), size and mtime (epoch ms), sorted by name, up to 500 a page (pass `cursor` from the reply for the next). path \"\" lists the folders that device lets agents reach. Paths are absolute on that device. Computers expose the home folder and project folders (credentials, keys, browser profiles and Zeron's own data are never shown); a phone exposes only the folders its person granted.",
            input_schema: json!({ "type": "object", "properties": {
                "device": device,
                "path": { "type": "string", "description": "Absolute folder path on the device; \"\" for the reachable roots." },
                "cursor": { "type": "string" }
            }, "required": ["path"] }),
        },
        ToolDef {
            name: STAT,
            description: "One file or folder on a device: kind, size and mtime (epoch ms).",
            input_schema: json!({ "type": "object", "properties": { "device": device, "path": { "type": "string" } }, "required": ["path"] }),
        },
        ToolDef {
            name: COPY,
            description: "Copy a file (or, with recursive, a folder) from one device to another, in chunks over Zeron's device relay: no one has to drive a screen. Works between any two of the account's devices, computers and the phone, and runs from this machine whichever ends they are. Every file is checked by sha256 before it appears under its name, a dropped link leaves a partial that the same call resumes from, and a destination that already holds identical bytes is left alone (outcome `identical`). It never replaces a different file unless overwrite is true. Limits: 2 GiB a file, 20 GiB a call. Waits up to 40 s: a bigger transfer answers state `running` with a transferId, and device_file_copy_status follows it. to_path is the destination file, or a folder ending in / to keep the name. The phone may ask its person to confirm a write.",
            input_schema: json!({ "type": "object", "properties": {
                "from_device": { "type": "string", "description": "Source device id or name. Omit for this machine." },
                "from_path": { "type": "string" },
                "to_device": { "type": "string", "description": "Destination device id or name. Omit for this machine." },
                "to_path": { "type": "string" },
                "overwrite": { "type": "boolean", "description": "Replace a different file at to_path. Default false." },
                "recursive": { "type": "boolean", "description": "Copy a folder and everything under it into to_path (missing folders are made). Default false." }
            }, "required": ["from_path", "to_path"] }),
        },
        ToolDef {
            name: COPY_STATUS,
            description: "Where a device_file_copy stands: state (running, done, failed), bytes_done of bytes_total, the file in progress, and per-file results with outcome, sha256 and any error. Waits up to 40 s for a running copy to finish. A failed copy that left a partial resumes when device_file_copy is called again with the same arguments.",
            input_schema: json!({ "type": "object", "properties": { "transfer_id": { "type": "string" } }, "required": ["transfer_id"] }),
        },
    ]
}

fn plain(error: RpcError) -> String {
    match error {
        RpcError::Failed(message) => message,
        RpcError::UnknownMethod(_) => {
            "That device's Zeron doesn't have device files yet; update Zeron on it to use them."
                .into()
        }
        other => other.to_string(),
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

impl Tools {
    /// A device key as the engine addresses it: `None` for this machine.
    async fn files_device(&self, key: Option<&str>) -> Result<Option<String>, String> {
        let Some(key) = key else { return Ok(None) };
        let id = self
            .zeron
            .resolve_device_id(Some(key))
            .await
            .map_err(|e| format!("{e:#}"))?;
        let local = self
            .zeron
            .local_device_id()
            .await
            .map_err(|e| format!("{e:#}"))?;
        Ok((id != local).then_some(id))
    }

    pub(super) async fn device_files(&self, tool: &str, args: Value) -> Result<Value, String> {
        let args = if args.is_null() { json!({}) } else { args };
        if !args.is_object() {
            return Err("invalid arguments: expected an object".into());
        }
        match tool {
            LIST | STAT => {
                let mut params =
                    json!({ "path": args.get("path").and_then(Value::as_str).unwrap_or_default() });
                if tool == LIST
                    && let Some(cursor) = text(&args, "cursor")
                {
                    params["cursor"] = json!(cursor);
                }
                if let Some(target) = self.files_device(text(&args, "device")).await? {
                    params["targetDeviceId"] = json!(target);
                }
                let method = if tool == LIST {
                    methods::DEVICE_FILE_LIST
                } else {
                    methods::DEVICE_FILE_STAT
                };
                self.zeron.call_rpc(method, params).await.map_err(plain)
            }
            COPY => {
                let need = |key: &str| text(&args, key).ok_or_else(|| format!("{key} is required"));
                let mut params = json!({
                    "fromPath": need("from_path")?,
                    "toPath": need("to_path")?,
                    "overwrite": args.get("overwrite").and_then(Value::as_bool).unwrap_or(false),
                    "recursive": args.get("recursive").and_then(Value::as_bool).unwrap_or(false),
                });
                if let Some(id) = self.files_device(text(&args, "from_device")).await? {
                    params["fromDeviceId"] = json!(id);
                }
                if let Some(id) = self.files_device(text(&args, "to_device")).await? {
                    params["toDeviceId"] = json!(id);
                }
                let started = self
                    .zeron
                    .call_rpc(methods::DEVICE_FILE_COPY, params)
                    .await
                    .map_err(plain)?;
                let id = started["transferId"]
                    .as_str()
                    .ok_or("the engine gave no transferId")?
                    .to_owned();
                self.follow_copy(&id).await
            }
            _ => {
                let id = text(&args, "transfer_id")
                    .ok_or("transfer_id is required")?
                    .to_owned();
                self.follow_copy(&id).await
            }
        }
    }

    /// The copy's status once it ends, or `running` after [`WAIT`].
    async fn follow_copy(&self, id: &str) -> Result<Value, String> {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let status = self
                .zeron
                .call_rpc(
                    methods::DEVICE_FILE_COPY_STATUS,
                    json!({ "transferId": id }),
                )
                .await
                .map_err(plain)?;
            if status["state"] != "running" || tokio::time::Instant::now() >= deadline {
                return Ok(status);
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tools;
    use crate::zeron::{Origin, Zeron};
    use async_trait::async_trait;
    use std::sync::Arc;
    use zeron_rpc::{RpcReply, RpcService, memory_client};

    /// An engine that answers the device-file methods with canned replies and records the params.
    struct Engine {
        seen: std::sync::Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl RpcService for Engine {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            self.seen
                .lock()
                .unwrap()
                .push((method.into(), params.clone()));
            let reply = match method {
                methods::LOCAL_DEVICE => json!({ "deviceId": "dev-pc" }),
                methods::WATCH_DEVICES => {
                    return Ok(RpcReply::Stream(Box::pin(futures::stream::iter([json!([
                        { "id": "dev-pc", "name": "PC", "platform": "windows", "lastSeenAt": null },
                        { "id": "dev-phone", "name": "Pixel", "platform": "android", "lastSeenAt": null }
                    ])]))));
                }
                methods::DEVICE_FILE_LIST => {
                    json!({ "entries": [{ "name": "a.m4a", "path": "/x/a.m4a", "kind": "file", "size": 3, "mtime": 1 }] })
                }
                methods::DEVICE_FILE_STAT => {
                    json!({ "name": "a", "kind": "file", "size": 1, "mtime": 2 })
                }
                methods::DEVICE_FILE_COPY => json!({ "transferId": "t1" }),
                methods::DEVICE_FILE_COPY_STATUS => {
                    json!({ "transferId": "t1", "state": "done", "bytesDone": 3, "bytesTotal": 3, "files": [] })
                }
                other => return Err(RpcError::UnknownMethod(other.into())),
            };
            Ok(RpcReply::Value(reply))
        }
    }

    fn tools() -> (Tools, Arc<Engine>) {
        let engine = Arc::new(Engine {
            seen: Default::default(),
        });
        (
            Tools::new(Arc::new(Zeron::with_client(
                memory_client(engine.clone()),
                Origin::default(),
            ))),
            engine,
        )
    }

    #[test]
    fn the_catalog_is_well_formed() {
        for tool in catalog() {
            assert!(NAMES.contains(&tool.name));
            assert_eq!(tool.input_schema["type"], "object");
            assert!(!tool.description.is_empty());
        }
        assert_eq!(catalog().len(), NAMES.len());
    }

    #[tokio::test]
    async fn a_listing_on_another_device_is_addressed_to_it_and_this_machine_is_not() {
        let (tools, engine) = tools();
        tools
            .call(LIST, json!({ "device": "Pixel", "path": "" }))
            .await
            .unwrap();
        tools
            .call(LIST, json!({ "device": "PC", "path": "/home" }))
            .await
            .unwrap();
        let seen = engine.seen.lock().unwrap();
        let lists: Vec<_> = seen
            .iter()
            .filter(|(m, _)| m == methods::DEVICE_FILE_LIST)
            .collect();
        assert_eq!(lists[0].1["targetDeviceId"], "dev-phone");
        assert!(lists[1].1.get("targetDeviceId").is_none());
    }

    #[tokio::test]
    async fn a_copy_names_both_devices_and_returns_the_finished_status() {
        let (tools, engine) = tools();
        let done = tools
            .call(COPY, json!({ "from_device": "Pixel", "from_path": "/storage/emulated/0/Recordings/Call/", "to_path": "C:/rec/", "recursive": true }))
            .await
            .unwrap();
        assert_eq!(done["state"], "done");
        let seen = engine.seen.lock().unwrap();
        let (_, params) = seen
            .iter()
            .find(|(m, _)| m == methods::DEVICE_FILE_COPY)
            .unwrap();
        assert_eq!(params["fromDeviceId"], "dev-phone");
        assert!(params.get("toDeviceId").is_none());
        assert_eq!(params["recursive"], true);
        assert_eq!(params["overwrite"], false);
    }

    #[tokio::test]
    async fn a_copy_without_paths_is_refused_before_asking_the_engine() {
        let (tools, engine) = tools();
        assert!(
            tools
                .call(COPY, json!({ "from_path": "/a" }))
                .await
                .unwrap_err()
                .contains("to_path")
        );
        assert!(
            engine
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|(m, _)| m != methods::DEVICE_FILE_COPY)
        );
    }
}
