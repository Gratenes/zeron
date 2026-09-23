//! Remote-listener behavior against a dev-mode authorizer: every request is
//! credential-gated, native clients authenticate with a Bearer on the
//! handshake, and browser upgrades authenticate with the first-frame `Auth`
//! envelope.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use zeron_engine::remote_access::{
    LoadedSettings, NetworkOptions, RemoteAccessSettings, load, resolve, save,
};
use zeron_engine::remote_auth::RemoteAuthorizer;
use zeron_engine::serve_engine_remote;
use zeron_rpc::{RpcError, RpcReply, RpcService, connect_ws, connect_ws_authenticated};

struct Echo;

#[async_trait]
impl RpcService for Echo {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        match method {
            "Echo" => Ok(RpcReply::Value(params)),
            other => Err(RpcError::UnknownMethod(other.to_owned())),
        }
    }
}

async fn start() -> zeron_engine::EngineListener {
    serve_engine_remote(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(Echo) as Arc<dyn RpcService>,
        Arc::new(RemoteAuthorizer::new(None)),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_is_credential_gated() {
    let mut listener = start().await;
    let http = reqwest::Client::new();
    let base = format!("http://{}/health", listener.address);

    let anonymous = http.get(&base).send().await.unwrap();
    assert_eq!(
        anonymous.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "remote requests without a bearer are refused"
    );

    let authenticated = http
        .get(&base)
        .bearer_auth("dev-user")
        .send()
        .await
        .unwrap();
    assert_eq!(authenticated.status(), reqwest::StatusCode::OK);
    let body: Value = authenticated.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    listener.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_bearer_dial_serves_rpc() {
    let mut listener = start().await;
    let url = format!("ws://{}", listener.address);

    let rpc = connect_ws_authenticated(&url, "dev-user").await.unwrap();
    let reply = rpc.call("Echo", json!({"hello": "world"})).await.unwrap();
    assert_eq!(reply, json!({"hello": "world"}));
    assert!(rpc.call("Nope", json!({})).await.is_err());
    listener.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anonymous_native_dial_is_refused_before_any_rpc() {
    let mut listener = start().await;
    let url = format!("ws://{}", listener.address);

    // The handshake succeeds (the browser path needs it to); the client's
    // first frame is an RPC call, not an `Auth` envelope, so the session is
    // refused and the pending call fails instead of reaching the service.
    let rpc = connect_ws(&url).await.unwrap();
    assert!(rpc.call("Echo", json!({"secret": 1})).await.is_err());
    listener.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browser_first_frame_envelope_authenticates_and_serves_rpc() {
    let mut listener = start().await;
    let url = format!("ws://{}", listener.address);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    use tokio_tungstenite::tungstenite::Message;
    ws.send(Message::Text(json!({"auth": "dev-user"}).to_string()))
        .await
        .unwrap();
    ws.send(Message::Text(
        json!({"id": 1, "method": "Echo", "params": {"a": 2}}).to_string(),
    ))
    .await
    .unwrap();

    let reply = loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("reply within timeout")
            .expect("socket open")
            .expect("frame decoded");
        if let Message::Text(text) = frame {
            let value: Value = serde_json::from_str(text.as_str()).unwrap();
            if value.get("id") == Some(&json!(1)) {
                break value;
            }
        }
    };
    assert_eq!(reply["ok"], json!({"a": 2}));
    listener.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_first_frame_envelope_closes_the_socket() {
    let mut listener = start().await;
    let url = format!("ws://{}", listener.address);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    use tokio_tungstenite::tungstenite::Message;
    ws.send(Message::Text(json!({"auth": ""}).to_string()))
        .await
        .unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("close within timeout")
        .expect("socket closes after a refused credential");
    match frame {
        Ok(Message::Close(Some(frame))) => {
            assert_eq!(frame.code, CloseCode::Library(4401));
        }
        other => panic!("expected a 4401 close, got: {other:?}"),
    }
    listener.stop().await;
}

#[test]
fn remote_access_settings_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let settings = RemoteAccessSettings {
        enabled: true,
        bind_address: "127.0.0.1:12345".parse().unwrap(),
    };
    save(dir.path(), &settings).unwrap();
    let loaded = load(dir.path());
    assert!(loaded.stored);
    assert!(loaded.error.is_none());
    assert!(loaded.settings.enabled);
    assert_eq!(
        loaded.settings.bind_address,
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap()
    );
}

#[test]
fn remote_access_resolve_reports_sources_and_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let untouched = load(dir.path());
    assert!(!resolve(&untouched, &NetworkOptions::default()).enabled);

    let flag = NetworkOptions {
        flag: Some(true),
        ..Default::default()
    };
    let status = resolve(&untouched, &flag);
    assert!(status.enabled);
    assert_eq!(status.source, "--network");

    let environment = NetworkOptions {
        environment: Some(true),
        ..Default::default()
    };
    let status = resolve(&untouched, &environment);
    assert!(status.enabled);
    assert_eq!(status.source, "ZERON_NETWORK");

    // The settings file alone enables remote access.
    save(dir.path(), &RemoteAccessSettings::default()).unwrap();
    let stored = load(dir.path());
    let stored_enabled = LoadedSettings {
        settings: RemoteAccessSettings {
            enabled: true,
            ..Default::default()
        },
        stored: true,
        error: None,
    };
    let status = resolve(&stored_enabled, &NetworkOptions::default());
    assert!(status.enabled);
    assert_eq!(status.source, "remote-access.json");

    // Disagreeing sources stay local rather than guessing a winner.
    let conflicting = NetworkOptions {
        flag: Some(false),
        ..Default::default()
    };
    let status = resolve(&stored_enabled, &conflicting);
    assert!(!status.enabled);
    assert!(status.error.is_some());
    let _ = stored;
}
