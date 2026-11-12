//! `PhoneCall`: an agent's phone tool, forwarded by its engine to the phone through the phone's
//! own device room (a minimal in-memory room stands in for the edge DO), answered by a mock phone.

// tungstenite's `accept_hdr_async` callback signature fixes the Err type as a full
// `Response` — its size is not ours to shrink.
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::handshake::server::{
    Request as WsRequest, Response as WsResponse,
};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_proto::HarnessId;
use zeron_rpc::{
    DeviceFrameHeader, HostRelay, HostRelayConfig, LinkCache, LinkCacheConfig, RpcError, RpcReply,
    RpcService, StaticToken, decode_device_frame, encode_device_frame, methods,
};

// ---------------------------------------------------------------------------
// Minimal in-memory device room (route-only subset of the DO semantics)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RelayState {
    host: Option<mpsc::UnboundedSender<Vec<u8>>>,
    clients: HashMap<String, mpsc::UnboundedSender<Vec<u8>>>,
}

async fn fake_device_room() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
    let url = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    let state = Arc::new(Mutex::new(RelayState::default()));
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let mut uri = String::new();
                let Ok(ws) = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |req: &WsRequest, res: WsResponse| {
                        uri = req.uri().to_string();
                        Ok(res)
                    },
                )
                .await
                else {
                    return;
                };
                let query = uri.split_once('?').map(|(_, q)| q).unwrap_or("");
                let is_host = query.contains("role=host");
                let conn_id = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("connId="))
                    .unwrap_or("anon")
                    .to_string();
                let (mut sink, mut ws_stream) = ws.split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
                {
                    let mut st = state.lock().expect("lock");
                    if is_host {
                        st.host = Some(tx);
                    } else {
                        st.clients.insert(conn_id.clone(), tx);
                    }
                }
                let writer = tokio::spawn(async move {
                    while let Some(bytes) = rx.recv().await {
                        if sink.send(WsMessage::Binary(bytes)).await.is_err() {
                            break;
                        }
                    }
                });
                while let Some(Ok(message)) = ws_stream.next().await {
                    let WsMessage::Binary(bytes) = message else {
                        continue;
                    };
                    let Ok((header, payload)) = decode_device_frame(&bytes) else {
                        break;
                    };
                    let st = state.lock().expect("lock");
                    if is_host {
                        let Some(to) = header.to else { continue };
                        if let Some(client) = st.clients.get(&to) {
                            let stripped = DeviceFrameHeader::new(header.s, header.k);
                            let _ = client
                                .send(encode_device_frame(&stripped, &payload).expect("encode"));
                        }
                    } else if let Some(host) = &st.host {
                        let mut routed = DeviceFrameHeader::new(header.s, header.k);
                        routed.from = Some(conn_id.clone());
                        let _ = host.send(encode_device_frame(&routed, &payload).expect("encode"));
                    }
                }
                writer.abort();
            });
        }
    });
    (url, task)
}

fn assemble(dir: &std::path::Path, device_id: &str) -> EngineCore {
    std::fs::create_dir_all(dir).expect("create data dir");
    std::fs::write(dir.join("device-id"), device_id).expect("write device id");
    EngineCore::assemble(dir, Arc::new(HarnessRegistry::new()), HarnessId::Mock, None).expect("engine assembles")
}

/// A device on the account, as the workspace doc lists it.
fn device_row(id: &str, version: &str, seen_hours_ago: i64) -> zeron_proto::Device {
    zeron_proto::Device {
        id: id.into(),
        name: format!("{id}-name"),
        platform: "linux".into(),
        last_seen_at: Some(chrono::Utc::now() - chrono::Duration::hours(seen_hours_ago)),
        created_at: None,
        version: Some(version.into()),
        cursor_sdk_version: None,
        capabilities: Vec::new(),
    }
}

fn asker(dir: &std::path::Path, relay_url: &str) -> EngineCore {
    let core = assemble(dir, "asking-device");
    let mut link_config =
        LinkCacheConfig::new(relay_url.to_string(), Arc::new(StaticToken("test-user".into())));
    link_config.probe_timeout = Duration::from_secs(2);
    core.set_links(LinkCache::new(link_config));
    core
}

// ---------------------------------------------------------------------------
// PhoneCall: an agent's phone tool, through the phone's own room
// ---------------------------------------------------------------------------

/// A phone as an agent's engine sees it: answers the dial's own probes, then `PhoneCall` by
/// echoing the call back (what the phone was asked, and by whom), the way a real phone's reply
/// names what it did. `old` is a phone whose Zeron predates the tools.
struct MockPhone {
    old: bool,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl RpcService for MockPhone {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::LIST_HARNESSES => RpcReply::value(&serde_json::json!([])),
            methods::ENGINE_READY => RpcReply::value(&serde_json::json!({ "ready": true })),
            methods::PHONE_CALL if !self.old => {
                self.calls.lock().unwrap().push(params.clone());
                if params["tool"] == "refuse" {
                    return Err(RpcError::Failed("Reading the screen is switched off on this phone".into()));
                }
                RpcReply::value(&serde_json::json!({ "tool": params["tool"], "args": params["args"], "origin": params["origin"] }))
            }
            other => Err(RpcError::UnknownMethod(other.into())),
        }
    }
}

fn spawn_phone(relay_url: &str, device: &str, old: bool) -> (HostRelay, Arc<Mutex<Vec<serde_json::Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut config = HostRelayConfig::new(relay_url, device, Arc::new(StaticToken("test-user".into())));
    // A phone's own, slower keepalive.
    config.keepalive = Some(Duration::from_secs(60));
    let host = HostRelay::spawn(config, Arc::new(MockPhone { old, calls: calls.clone() }), Arc::new(|_| true));
    (host, calls)
}

fn phone_row(id: &str) -> zeron_proto::Device {
    zeron_proto::Device { platform: "android".into(), capabilities: vec!["phone-tools-v1".into()], ..device_row(id, "0.3.0", 5 * 24) }
}

async fn phone_call(client: &zeron_rpc::RpcClient, params: serde_json::Value) -> Result<serde_json::Value, RpcError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        match client.call(methods::PHONE_CALL, params.clone()).await {
            // The phone's host may not have joined its room yet: the engine waits for it, too.
            Err(error) if error.to_string().contains("offline") && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            other => return other,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_call_reaches_the_phone_even_when_its_registry_presence_is_dark() {
    let (relay_url, _relay) = fake_device_room().await;
    let dirs = tempfile::tempdir().expect("tempdir");
    // A phone rests its workspace sockets to spare its battery: the registry has had no
    // presence from it for days. The dial gate must not take that for "away".
    let mut link_config = LinkCacheConfig::new(relay_url.clone(), Arc::new(StaticToken("test-user".into())));
    link_config.probe_timeout = Duration::from_secs(2);
    link_config.liveness = Some(Arc::new(|_| zeron_rpc::PeerLiveness::Dark));
    let a = assemble(&dirs.path().join("a"), "asking-device");
    a.set_links(LinkCache::new(link_config));
    a.workspace.upsert_device_row(&phone_row("pixel"));
    let (_phone, calls) = spawn_phone(&relay_url, "pixel", false);
    let client = zeron_rpc::memory_client(a.rpc_service());

    let params = serde_json::json!({
        "targetDeviceId": "pixel", "tool": "screen_read", "args": { "max_nodes": 5 },
        "origin": { "chatId": "chat-1", "chatTitle": "Fix it" },
    });
    let reply = phone_call(&client, params).await.expect("the phone answers");
    assert_eq!(reply["tool"], "screen_read");
    assert_eq!(reply["args"], serde_json::json!({ "max_nodes": 5 }));
    assert_eq!(reply["origin"]["chatTitle"], "Fix it", "the phone is told who asked");
    assert_eq!(calls.lock().unwrap().len(), 1);

    // What the phone refuses comes back as its own words, named by the device.
    let error = client
        .call(methods::PHONE_CALL, serde_json::json!({ "targetDeviceId": "pixel", "tool": "refuse", "args": {} }))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("switched off on this phone"), "{error}");
    assert!(error.to_string().contains("pixel-name"), "names the device: {error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_call_says_when_the_phone_predates_the_tools() {
    let (relay_url, _relay) = fake_device_room().await;
    let dirs = tempfile::tempdir().expect("tempdir");
    let a = asker(&dirs.path().join("a"), &relay_url);
    a.workspace.upsert_device_row(&phone_row("old-pixel"));
    let (_phone, _calls) = spawn_phone(&relay_url, "old-pixel", true);
    let client = zeron_rpc::memory_client(a.rpc_service());
    let error = phone_call(&client, serde_json::json!({ "targetDeviceId": "old-pixel", "tool": "ping", "args": {} }))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("doesn't run Zeron's phone tools"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_call_is_refused_without_a_target_or_for_a_stranger_or_this_device() {
    let (relay_url, _relay) = fake_device_room().await;
    let dirs = tempfile::tempdir().expect("tempdir");
    let a = asker(&dirs.path().join("a"), &relay_url);
    let _stranger = spawn_phone(&relay_url, "stranger", false);
    let client = zeron_rpc::memory_client(a.rpc_service());
    let started = std::time::Instant::now();
    for (params, expect) in [
        (serde_json::json!({ "tool": "ping" }), "needs targetDeviceId"),
        (serde_json::json!({ "targetDeviceId": "stranger", "tool": "ping" }), "isn't one of this account's devices"),
        (serde_json::json!({ "targetDeviceId": "asking-device", "tool": "ping" }), "isn't a phone"),
    ] {
        let error = client.call(methods::PHONE_CALL, params).await.unwrap_err();
        assert!(error.to_string().contains(expect), "{expect}: {error}");
    }
    assert!(started.elapsed() < Duration::from_secs(2), "refused without asking anyone");
}

/// The link to a phone is let go after a quiet spell, so its keepalive doesn't wake the radio.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_link_to_a_phone_is_dropped_when_it_has_been_quiet() {
    let (relay_url, _relay) = fake_device_room().await;
    let dirs = tempfile::tempdir().expect("tempdir");
    let a = asker(&dirs.path().join("a"), &relay_url);
    a.workspace.upsert_device_row(&phone_row("pixel"));
    let (_phone, _calls) = spawn_phone(&relay_url, "pixel", false);
    let client = zeron_rpc::memory_client(a.rpc_service());
    phone_call(&client, serde_json::json!({ "targetDeviceId": "pixel", "tool": "ping", "args": {} }))
        .await
        .expect("answered");
    // Paused time can't be used with real sockets: ask the engine whether its idle timer is armed.
    assert!(zeron_engine::phone_link_idle_pending("pixel"), "a quiet-spell timer is running for the phone");
}
