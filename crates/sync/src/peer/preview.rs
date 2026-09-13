//! Authenticated preview catalog and opaque Mux-frame relay.
//!
//! The engine mounts [`router`] beside the durable peer routes and installs a
//! [`PeerPrincipal`](super::PeerPrincipal) extension. Connections are isolated
//! by authenticated profile and requested organization. A device may only
//! register under its authenticated device ID; every binary frame is rewritten
//! with that source ID before forwarding, so target and source assignment
//! cannot be forged by clients.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use axum::{
    Router,
    extract::{
        Extension, Path, Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::PeerPrincipal;

const MAX_DEVICE_ID: usize = 128;
const MAX_MUX_FRAME: usize = 8197;
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_SERVICES: usize = 256;

#[derive(Clone, Default)]
pub struct PreviewState(Arc<Inner>);

#[derive(Default)]
struct Inner {
    connections: Mutex<HashMap<Key, Connection>>,
    next_id: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    profile: String,
    org: String,
    device: String,
}

struct Connection {
    id: u64,
    tx: mpsc::Sender<Message>,
    revoked: watch::Sender<bool>,
    catalog: Option<Vec<Value>>,
}

#[derive(Deserialize)]
struct PreviewQuery {
    device: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientMessage {
    Catalog { services: Vec<Value> },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceIdentity {
    id: String,
    device_id: String,
    hostname: String,
    project_cwd: String,
    name: String,
}

/// Build `/preview/{org}/ws`. Authentication middleware must be layered by the
/// caller and provide `PeerPrincipal` for every request.
pub fn router(state: PreviewState) -> Router {
    Router::new()
        .route("/preview/{org}/ws", get(preview_ws))
        .with_state(state)
}

impl PreviewState {
    /// Immediately revoke every preview socket authenticated as `device` in
    /// `profile`. Active streams close and other devices receive catalog
    /// `gone` events. This is intentionally synchronous for use from the same
    /// trusted-device revocation path as the other peer protocols.
    pub fn revoke_device(&self, profile: &str, device: &str) {
        let (removed, recipients) = {
            let mut connections = self
                .0
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let keys: Vec<_> = connections
                .keys()
                .filter(|key| key.profile == profile && key.device == device)
                .cloned()
                .collect();
            let mut removed = Vec::new();
            for key in keys {
                if let Some(connection) = connections.remove(&key) {
                    connection.revoked.send_replace(true);
                    removed.push(key);
                }
            }
            let recipients: Vec<_> = connections
                .iter()
                .map(|(key, connection)| (key.clone(), connection.tx.clone()))
                .collect();
            (removed, recipients)
        };
        for gone in removed {
            let message = gone_message(&gone.device);
            for (key, tx) in &recipients {
                if key.profile == gone.profile && key.org == gone.org {
                    // A congested catalog consumer is disconnected by its own
                    // bounded queue rather than accumulating revocation work.
                    if tx.try_send(message.clone()).is_err() {
                        self.cancel(key);
                    }
                }
            }
        }
    }

    fn cancel(&self, key: &Key) {
        if let Some(connection) = self
            .0
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
        {
            connection.revoked.send_replace(true);
        }
    }

    fn join(
        &self,
        key: Key,
    ) -> (
        u64,
        mpsc::Receiver<Message>,
        watch::Receiver<bool>,
        Vec<(String, Vec<Value>)>,
    ) {
        let id = self
            .0
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        let (tx, rx) = mpsc::channel(64);
        let (revoked, revoked_rx) = watch::channel(false);
        let mut connections = self
            .0
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = connections.remove(&key) {
            previous.revoked.send_replace(true);
            // A replacement socket starts fresh Mux stream IDs. Tell every
            // peer to discard the previous pair before this device advertises
            // its new catalog or sends application frames.
            let gone = gone_message(&key.device);
            for (other, connection) in connections.iter() {
                if other.profile == key.profile && other.org == key.org {
                    if connection.tx.try_send(gone.clone()).is_err() {
                        connection.revoked.send_replace(true);
                    }
                }
            }
        }
        let catalogs = connections
            .iter()
            .filter(|(other, connection)| {
                other.profile == key.profile
                    && other.org == key.org
                    && other.device != key.device
                    && connection.catalog.is_some()
            })
            .map(|(other, connection)| (other.device.clone(), connection.catalog.clone().unwrap()))
            .collect();
        connections.insert(
            key,
            Connection {
                id,
                tx,
                revoked,
                catalog: None,
            },
        );
        (id, rx, revoked_rx, catalogs)
    }

    async fn publish_catalog(
        &self,
        key: &Key,
        id: u64,
        services: Vec<Value>,
    ) -> Result<(), &'static str> {
        validate_catalog(&key.device, &services)?;
        let recipients = {
            let mut connections = self
                .0
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let Some(connection) = connections.get_mut(key).filter(|c| c.id == id) else {
                return Err("connection_replaced");
            };
            connection.catalog = Some(services.clone());
            connections
                .iter()
                .filter(|(other, _)| {
                    other.profile == key.profile
                        && other.org == key.org
                        && other.device != key.device
                })
                .map(|(_, connection)| connection.tx.clone())
                .collect::<Vec<_>>()
        };
        let message = Message::Text(
            json!({"type":"catalog", "device":key.device, "services":services})
                .to_string()
                .into(),
        );
        for recipient in recipients {
            recipient
                .send(message.clone())
                .await
                .map_err(|_| "peer_closed")?;
        }
        Ok(())
    }

    async fn relay(
        &self,
        key: &Key,
        id: u64,
        target: &str,
        payload: &[u8],
    ) -> Result<bool, &'static str> {
        if target == key.device || !valid_device(target) {
            return Err("invalid_target");
        }
        if !(5..=MAX_MUX_FRAME).contains(&payload.len()) {
            return Err("invalid_frame");
        }
        let recipient = {
            let connections = self
                .0
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if connections
                .get(key)
                .is_none_or(|connection| connection.id != id)
            {
                return Err("connection_replaced");
            }
            connections
                .get(&Key {
                    profile: key.profile.clone(),
                    org: key.org.clone(),
                    device: target.to_owned(),
                })
                .map(|connection| connection.tx.clone())
        };
        let Some(recipient) = recipient else {
            return Ok(false);
        };
        let envelope = encode_envelope(&key.device, payload).map_err(|_| "invalid_frame")?;
        recipient
            .send(Message::Binary(envelope.into()))
            .await
            .map_err(|_| "peer_closed")?;
        Ok(true)
    }

    fn leave(&self, key: &Key, id: u64) -> Vec<(mpsc::Sender<Message>, watch::Sender<bool>)> {
        let mut connections = self
            .0
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if connections
            .get(key)
            .is_none_or(|connection| connection.id != id)
        {
            return Vec::new();
        }
        connections.remove(key);
        connections
            .iter()
            .filter(|(other, _)| other.profile == key.profile && other.org == key.org)
            .map(|(_, connection)| (connection.tx.clone(), connection.revoked.clone()))
            .collect()
    }
}

async fn preview_ws(
    ws: WebSocketUpgrade,
    State(state): State<PreviewState>,
    Extension(principal): Extension<PeerPrincipal>,
    Path(org): Path<String>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, (StatusCode, axum::Json<Value>)> {
    if !valid_device(&query.device)
        || query.device != principal.device_id
        || org.is_empty()
        || org.len() > 256
    {
        return Err((
            StatusCode::FORBIDDEN,
            axum::Json(json!({"error":"forbidden"})),
        ));
    }
    Ok(ws
        .on_upgrade(move |socket| preview_socket(socket, state, principal, org))
        .into_response())
}

async fn preview_socket(
    socket: WebSocket,
    state: PreviewState,
    principal: PeerPrincipal,
    org: String,
) {
    let key = Key {
        profile: principal.profile_id,
        org,
        device: principal.device_id,
    };
    let (id, mut outgoing, mut revoked, catalogs) = state.join(key.clone());
    let (mut sink, mut incoming) = socket.split();
    for (device, services) in catalogs {
        let message = Message::Text(
            json!({"type":"catalog", "device":device, "services":services})
                .to_string()
                .into(),
        );
        if sink.send(message).await.is_err() {
            let recipients = state.leave(&key, id);
            broadcast_gone(recipients, &key.device);
            return;
        }
    }
    loop {
        tokio::select! {

            // Revocation removes the connection and therefore closes its
            // outgoing queue at the same instant. Always service the watch
            // notification first so code 4403 cannot lose that readiness race.
            biased;
            changed = revoked.changed() => {
                if changed.is_err() || *revoked.borrow() {
                    if sink
                        .send(Message::Close(Some(CloseFrame {
                            code: 4403,
                            reason: "device revoked".into(),
                        })))
                        .await
                        .is_ok()
                    {
                        // Complete the WebSocket close handshake before the
                        // upgraded TCP stream is dropped. Otherwise clients
                        // can observe a transport reset instead of code 4403.
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(1),
                            incoming.next(),
                        )
                        .await;
                    }
                    break;
                }
            }
            output = outgoing.recv() => match output {
                Some(message) => if sink.send(message).await.is_err() { break },
                None => break,
            },
            input = incoming.next() => {
                let Some(Ok(message)) = input else { break };
                match message {
                    Message::Text(text) if text.as_str() == "ping" => {
                        if sink.send(Message::Text("pong".into())).await.is_err() { break }
                    }
                    Message::Text(text) => {
                        if text.len() > MAX_CATALOG_BYTES { break }
                        let Ok(ClientMessage::Catalog { services }) = serde_json::from_str(&text) else { break };
                        if state.publish_catalog(&key, id, services).await.is_err() { break }
                    }
                    Message::Binary(bytes) => {
                        let Ok((target, payload)) = decode_envelope(&bytes) else { break };
                        match state.relay(&key, id, target, payload).await {
                            Ok(true) => {},
                            Ok(false) => {
                                if sink.send(gone_message(target)).await.is_err() { break }
                            }
                            Err(_) => break,
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(bytes) => {
                        if sink.send(Message::Pong(bytes)).await.is_err() { break }
                    }
                    _ => {},
                }
            }
        }
    }
    let recipients = state.leave(&key, id);
    broadcast_gone(recipients, &key.device);
}

fn broadcast_gone(recipients: Vec<(mpsc::Sender<Message>, watch::Sender<bool>)>, device: &str) {
    let message = gone_message(device);
    for (recipient, revoked) in recipients {
        // Never leave stale catalog routes behind. If the bounded queue is
        // full, close that stalled consumer so its reconnect obtains a fresh
        // catalog snapshot instead of silently dropping `gone`.
        if recipient.try_send(message.clone()).is_err() {
            revoked.send_replace(true);
        }
    }
}

fn gone_message(device: &str) -> Message {
    Message::Text(json!({"type":"gone", "device":device}).to_string().into())
}

fn valid_device(device: &str) -> bool {
    !device.is_empty()
        && device.len() <= MAX_DEVICE_ID
        && device
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:@-".contains(&byte))
}

fn validate_catalog(device: &str, services: &[Value]) -> Result<(), &'static str> {
    if services.len() > MAX_SERVICES {
        return Err("too_many_services");
    }
    let mut ids = std::collections::HashSet::new();
    for value in services {
        let service: ServiceIdentity =
            serde_json::from_value(value.clone()).map_err(|_| "invalid_service")?;
        if service.device_id != device
            || service.id.is_empty()
            || service.id.len() > 128
            || !ids.insert(service.id)
            || service.hostname.len() > 255
            || !valid_hostname(&service.hostname)
            || service.project_cwd.len() > 4096
            || service.name.len() > 128
        {
            return Err("invalid_service");
        }
    }
    Ok(())
}

fn valid_hostname(host: &str) -> bool {
    let labels: Vec<_> = host.split('.').collect();
    labels.len() == 3
        && labels[2] == "localhost"
        && labels[..2].iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn encode_envelope(device: &str, payload: &[u8]) -> Result<Vec<u8>, ()> {
    if !valid_device(device) || !(5..=MAX_MUX_FRAME).contains(&payload.len()) {
        return Err(());
    }
    let mut output = Vec::with_capacity(2 + device.len() + payload.len());
    output.extend_from_slice(&(device.len() as u16).to_be_bytes());
    output.extend_from_slice(device.as_bytes());
    output.extend_from_slice(payload);
    Ok(output)
}

fn decode_envelope(bytes: &[u8]) -> Result<(&str, &[u8]), ()> {
    let length = u16::from_be_bytes(bytes.get(..2).ok_or(())?.try_into().unwrap()) as usize;
    if !(1..=MAX_DEVICE_ID).contains(&length) || bytes.len() < 2 + length + 5 {
        return Err(());
    }
    let device = std::str::from_utf8(&bytes[2..2 + length]).map_err(|_| ())?;
    let payload = &bytes[2 + length..];
    if !valid_device(device) || payload.len() > MAX_MUX_FRAME {
        return Err(());
    }
    Ok((device, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(profile: &str, org: &str, device: &str) -> Key {
        Key {
            profile: profile.into(),
            org: org.into(),
            device: device.into(),
        }
    }

    fn service(device: &str) -> Value {
        json!({
            "id": "known-service",
            "deviceId": device,
            "hostname": "laptop.project.localhost",
            "projectCwd": "/work/project",
            "name": "Vite"
        })
    }

    async fn serve_as(
        state: PreviewState,
        profile: &str,
        device: &str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(state).layer(Extension(PeerPrincipal {
            profile_id: profile.into(),
            device_id: device.into(),
        }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("ws://{address}"), task)
    }

    #[tokio::test]
    async fn real_websocket_relay_authenticates_routes_and_revokes() {
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let state = PreviewState::default();
        let (host_url, host_server) = serve_as(state.clone(), "profile", "host").await;
        let (viewer_url, viewer_server) = serve_as(state.clone(), "profile", "viewer").await;
        let (other_url, other_server) = serve_as(state.clone(), "other-profile", "other").await;
        let (mut host, _) =
            tokio_tungstenite::connect_async(format!("{host_url}/preview/org/ws?device=host"))
                .await
                .unwrap();
        let (mut viewer, _) =
            tokio_tungstenite::connect_async(format!("{viewer_url}/preview/org/ws?device=viewer"))
                .await
                .unwrap();
        let (mut other, _) =
            tokio_tungstenite::connect_async(format!("{other_url}/preview/org/ws?device=other"))
                .await
                .unwrap();

        host.send(WsMessage::Text(
            json!({"type":"catalog", "services":[service("host")]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let WsMessage::Text(catalog) = viewer.next().await.unwrap().unwrap() else {
            panic!("viewer did not receive host catalog")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&catalog).unwrap()["device"],
            "host"
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), other.next())
                .await
                .is_err(),
            "another profile observed the catalog"
        );

        let payload = [1, 0, 0, 0, 7];
        viewer
            .send(WsMessage::Binary(
                encode_envelope("host", &payload).unwrap().into(),
            ))
            .await
            .unwrap();
        let WsMessage::Binary(frame) = host.next().await.unwrap().unwrap() else {
            panic!("host did not receive relay frame")
        };
        let (source, forwarded) = decode_envelope(&frame).unwrap();
        assert_eq!(source, "viewer");
        assert_eq!(forwarded, payload);

        let unauthorized =
            tokio_tungstenite::connect_async(format!("{viewer_url}/preview/org/ws?device=forged"))
                .await
                .unwrap_err();
        assert!(
            matches!(unauthorized, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN)
        );

        state.revoke_device("profile", "viewer");
        let WsMessage::Close(close) = viewer.next().await.unwrap().unwrap() else {
            panic!("revoked viewer socket remained open")
        };
        assert_eq!(u16::from(close.unwrap().code), 4403);
        let WsMessage::Text(gone) = host.next().await.unwrap().unwrap() else {
            panic!("host did not receive revocation gone event")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&gone).unwrap(),
            json!({"type":"gone", "device":"viewer"})
        );

        let _ = other.close(None).await;
        host_server.abort();
        viewer_server.abort();
        other_server.abort();
    }

    #[tokio::test]
    async fn catalogs_and_frames_are_profile_and_org_isolated() {
        let state = PreviewState::default();
        let a = key("profile-a", "org", "device-a");
        let b = key("profile-a", "org", "device-b");
        let other_profile = key("profile-b", "org", "device-b");
        let other_org = key("profile-a", "other", "device-b");
        let (a_id, _a_rx, _, _) = state.join(a.clone());
        let (_b_id, mut b_rx, _, _) = state.join(b.clone());
        let (_, mut profile_rx, _, _) = state.join(other_profile);
        let (_, mut org_rx, _, _) = state.join(other_org);

        state
            .publish_catalog(&a, a_id, vec![service("device-a")])
            .await
            .unwrap();
        let Message::Text(catalog) = b_rx.recv().await.unwrap() else {
            panic!("catalog must be text")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&catalog).unwrap()["device"],
            "device-a"
        );
        assert!(profile_rx.try_recv().is_err());
        assert!(org_rx.try_recv().is_err());

        let payload = [1, 0, 0, 0, 1];
        assert!(state.relay(&a, a_id, "device-b", &payload).await.unwrap());
        let Message::Binary(frame) = b_rx.recv().await.unwrap() else {
            panic!("relay frame must be binary")
        };
        let (source, forwarded) = decode_envelope(&frame).unwrap();
        assert_eq!(source, "device-a");
        assert_eq!(forwarded, payload);
        assert!(!state.relay(&a, a_id, "missing", &payload).await.unwrap());
        assert!(state.relay(&a, a_id, "device-a", &payload).await.is_err());
    }

    #[tokio::test]
    async fn revocation_closes_the_device_and_publishes_gone() {
        let state = PreviewState::default();
        let a = key("profile", "org", "device-a");
        let b = key("profile", "org", "device-b");
        let (_a_id, _a_rx, mut a_revoked, _) = state.join(a);
        let (_b_id, mut b_rx, _, _) = state.join(b);

        state.revoke_device("profile", "device-a");
        a_revoked.changed().await.unwrap();
        assert!(*a_revoked.borrow());
        let Message::Text(gone) = b_rx.recv().await.unwrap() else {
            panic!("gone must be text")
        };
        let gone: Value = serde_json::from_str(&gone).unwrap();
        assert_eq!(gone, json!({"type":"gone", "device":"device-a"}));
    }

    #[tokio::test]
    async fn claimed_device_and_oversized_catalogs_are_rejected() {
        let state = PreviewState::default();
        let a = key("profile", "org", "device-a");
        let (id, _, _, _) = state.join(a.clone());
        assert!(
            state
                .publish_catalog(&a, id, vec![service("forged-device")])
                .await
                .is_err()
        );
        assert!(validate_catalog("device-a", &vec![service("device-a"); 257]).is_err());
        assert!(decode_envelope(&[0, 1, b'a', 1, 2, 3, 4]).is_err());
    }
}
