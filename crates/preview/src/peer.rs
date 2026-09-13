//! Bounded application multiplexing over the authenticated preview WebSocket.
//!
//! One relay socket is shared by all remote devices. Each device pair gets an
//! independent [`Mux`]; the relay only forwards opaque frames and stamps their
//! authenticated source device. Tailcat protects the HTTP/WebSocket connection
//! to the durable peer, while this layer supplies application authorization,
//! target assignment, flow control, and service-level containment.
use crate::mux::{Connector, Mux, Stream};
use anyhow::Context;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub const MAX_DEVICE_ID: usize = 128;
pub const MAX_MUX_FRAME: usize = 8197;

#[derive(Debug)]
pub struct OutgoingFrame {
    pub to: String,
    pub bytes: Vec<u8>,
}

struct Peer {
    mux: Mux,
    input: mpsc::Sender<Vec<u8>>,
    stop: CancellationToken,
}

struct Inner {
    device: String,
    connector: Arc<dyn Connector>,
    peers: Mutex<HashMap<String, Peer>>,
    output: mpsc::Sender<OutgoingFrame>,
    stop: CancellationToken,
}

/// Remote device Muxes backed by a single authenticated relay connection.
#[derive(Clone)]
pub struct Peers(Arc<Inner>);

impl Peers {
    pub fn new(
        device: String,
        connector: Arc<dyn Connector>,
        stop: CancellationToken,
    ) -> (Self, mpsc::Receiver<OutgoingFrame>) {
        let (output, receiver) = mpsc::channel(64);
        (
            Self(Arc::new(Inner {
                device,
                connector,
                peers: Mutex::new(HashMap::new()),
                output,
                stop,
            })),
            receiver,
        )
    }

    async fn ensure(&self, device: &str) -> anyhow::Result<Mux> {
        anyhow::ensure!(
            device != self.0.device && !device.is_empty() && device.len() <= MAX_DEVICE_ID,
            "invalid preview peer"
        );
        let mut peers = self.0.peers.lock().await;
        if let Some(peer) = peers.get(device).filter(|peer| !peer.mux.is_closed()) {
            return Ok(peer.mux.clone());
        }
        anyhow::ensure!(
            peers.len() < 16 || peers.contains_key(device),
            "too many preview peers"
        );
        if let Some(old) = peers.remove(device) {
            old.stop.cancel();
            old.mux.close();
        }
        let (input, receiver) = mpsc::channel(64);
        let stop = self.0.stop.child_token();
        let transport = Arc::new(RelayTransport {
            to: device.to_owned(),
            output: self.0.output.clone(),
            input: Mutex::new(receiver),
            stop: stop.clone(),
        });
        // Device IDs deterministically assign stream-ID parity. Both ends can
        // create the pair concurrently without a negotiation round trip.
        let mux = Mux::start(
            transport,
            self.0.connector.clone(),
            self.0.device.as_str() < device,
            stop.clone(),
        );
        peers.insert(
            device.to_owned(),
            Peer {
                mux: mux.clone(),
                input,
                stop,
            },
        );
        Ok(mux)
    }

    /// Deliver one source-stamped opaque frame received from the relay.
    pub async fn receive(&self, from: &str, bytes: Vec<u8>) -> anyhow::Result<()> {
        anyhow::ensure!(
            (5..=MAX_MUX_FRAME).contains(&bytes.len()),
            "invalid relayed preview frame"
        );
        let _ = self.ensure(from).await?;
        let input = self
            .0
            .peers
            .lock()
            .await
            .get(from)
            .map(|peer| peer.input.clone())
            .context("preview peer disappeared")?;
        tokio::select! {
            _ = self.0.stop.cancelled() => anyhow::bail!("preview networking stopped"),
            result = input.send(bytes) => result.context("preview peer closed"),
        }
    }

    pub async fn open(
        &self,
        device: &str,
        service: &str,
        websocket: bool,
    ) -> anyhow::Result<Stream> {
        let mux = self.ensure(device).await?;
        let result = tokio::time::timeout(Duration::from_secs(15), mux.open(service, websocket))
            .await
            .context("Preview host did not answer through the durable peer")?;
        if result.is_err() && mux.is_closed() {
            self.remove(device).await;
        }
        result
    }

    pub async fn remove(&self, device: &str) {
        if let Some(peer) = self.0.peers.lock().await.remove(device) {
            peer.stop.cancel();
            peer.mux.close();
        }
    }

    pub async fn clear(&self) {
        let peers = std::mem::take(&mut *self.0.peers.lock().await);
        for (_, peer) in peers {
            peer.stop.cancel();
            peer.mux.close();
        }
    }
}

struct RelayTransport {
    to: String,
    output: mpsc::Sender<OutgoingFrame>,
    input: Mutex<mpsc::Receiver<Vec<u8>>>,
    stop: CancellationToken,
}

#[async_trait::async_trait]
impl crate::mux::Transport for RelayTransport {
    async fn send(&self, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            (5..=MAX_MUX_FRAME).contains(&bytes.len()),
            "invalid preview mux frame"
        );
        tokio::select! {
            _ = self.stop.cancelled() => anyhow::bail!("preview peer closed"),
            result = self.output.send(OutgoingFrame { to: self.to.clone(), bytes: bytes.to_vec() }) => result.context("preview relay disconnected"),
        }
    }

    async fn receive(&self) -> anyhow::Result<Vec<u8>> {
        let mut input = self.input.lock().await;
        tokio::select! {
            _ = self.stop.cancelled() => anyhow::bail!("preview peer closed"),
            value = input.recv() => value.context("preview relay disconnected"),
        }
    }
}

/// Encode a target/source device plus one opaque Mux frame. The client writes
/// the destination; the relay decodes it and re-encodes the authenticated
/// source, so clients can never forge `from`.
pub fn encode_envelope(device: &str, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !device.is_empty() && device.len() <= MAX_DEVICE_ID && !device.contains('\0'),
        "invalid preview device"
    );
    anyhow::ensure!(
        (5..=MAX_MUX_FRAME).contains(&bytes.len()),
        "invalid preview mux frame"
    );
    let mut output = Vec::with_capacity(2 + device.len() + bytes.len());
    output.extend_from_slice(&(device.len() as u16).to_be_bytes());
    output.extend_from_slice(device.as_bytes());
    output.extend_from_slice(bytes);
    Ok(output)
}

pub fn decode_envelope(bytes: &[u8]) -> anyhow::Result<(&str, &[u8])> {
    anyhow::ensure!(bytes.len() >= 2, "invalid preview envelope");
    let length = u16::from_be_bytes(bytes[..2].try_into().unwrap()) as usize;
    anyhow::ensure!(
        (1..=MAX_DEVICE_ID).contains(&length) && bytes.len() >= 2 + length + 5,
        "invalid preview envelope"
    );
    let device = std::str::from_utf8(&bytes[2..2 + length])?;
    anyhow::ensure!(!device.contains('\0'), "invalid preview device");
    let payload = &bytes[2 + length..];
    anyhow::ensure!(payload.len() <= MAX_MUX_FRAME, "oversized preview frame");
    Ok((device, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mux::BoxIo;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Echo;
    #[async_trait::async_trait]
    impl Connector for Echo {
        async fn connect(&self, service: &str) -> anyhow::Result<BoxIo> {
            anyhow::ensure!(service == "service", "unknown service");
            let (client, mut server) = tokio::io::duplex(65536);
            tokio::spawn(async move {
                let (mut read, mut write) = tokio::io::split(&mut server);
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
            Ok(Box::new(client))
        }
    }

    #[tokio::test]
    async fn relay_pair_streams_both_directions_and_denies_unknown_services() {
        let stop = CancellationToken::new();
        let (a, mut a_out) = Peers::new("a".into(), Arc::new(Echo), stop.child_token());
        let (b, mut b_out) = Peers::new("b".into(), Arc::new(Echo), stop.child_token());
        let to_b = b.clone();
        let forward_a = tokio::spawn(async move {
            while let Some(frame) = a_out.recv().await {
                assert_eq!(frame.to, "b");
                to_b.receive("a", frame.bytes).await.unwrap();
            }
        });
        let to_a = a.clone();
        let forward_b = tokio::spawn(async move {
            while let Some(frame) = b_out.recv().await {
                assert_eq!(frame.to, "a");
                to_a.receive("b", frame.bytes).await.unwrap();
            }
        });
        for (peers, target) in [(&a, "b"), (&b, "a")] {
            let mut stream = peers.open(target, "service", false).await.unwrap();
            stream.write_all(b"through relay").await.unwrap();
            stream.shutdown().await.unwrap();
            let mut output = Vec::new();
            stream.read_to_end(&mut output).await.unwrap();
            assert_eq!(output, b"through relay");
            assert!(peers.open(target, "not-advertised", false).await.is_err());
        }
        stop.cancel();
        a.clear().await;
        b.clear().await;
        forward_a.abort();
        forward_b.abort();
    }

    #[test]
    fn envelope_is_bounded_and_target_assigned() {
        let payload = [7; 5];
        let encoded = encode_envelope("device-b", &payload).unwrap();
        assert_eq!(
            decode_envelope(&encoded).unwrap(),
            ("device-b", payload.as_slice())
        );
        assert!(encode_envelope("", &payload).is_err());
        assert!(encode_envelope("b", &[0; MAX_MUX_FRAME + 1]).is_err());
        assert!(decode_envelope(&[0, 2, b'a']).is_err());
    }
}
