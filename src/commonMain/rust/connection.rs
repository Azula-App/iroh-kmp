use std::sync::Arc;

use iroh::endpoint::{Connection, ConnectionError, VarInt};

use crate::error::IrohError;
use crate::stream::{IrohRecvStream, IrohSendStream, IrohStream};

/// An established QUIC connection to a peer, with no stream opened yet. Returned by
/// [`crate::IrohEndpoint::connect_conn`] / `connect_addr` / `connect_by_id` /
/// `accept_conn`; [`crate::IrohEndpoint::connect`] and `accept_next` build on top of this to
/// also open/accept the app's usual bidirectional stream.
#[derive(uniffi::Object)]
pub struct IrohConnection {
    inner: Connection,
}

impl IrohConnection {
    pub(crate) fn new(inner: Connection) -> Self {
        IrohConnection { inner }
    }
}

/// Whether a [`ConnPath`] runs directly over IP or via a relay server.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum PathKind {
    Ip,
    Relay,
}

/// A single network path making up a connection, as reported by [`IrohConnection::paths`].
#[derive(uniffi::Record)]
pub struct ConnPath {
    pub addr: String,
    pub kind: PathKind,
    pub is_selected: bool,
    pub rtt_ms: Option<u64>,
}

/// Maps a QUIC [`ConnectionError`] to an [`IrohError`], prefixing the message with a stable
/// `"closed: "` token when the error means the connection is closed (rather than e.g. a
/// transport-level failure), so the Kotlin side can string-match on it.
fn map_conn_err(e: ConnectionError) -> IrohError {
    use ConnectionError::*;
    match e {
        LocallyClosed | ApplicationClosed(_) | ConnectionClosed(_) => {
            IrohError::Generic { msg: format!("closed: {e}") }
        }
        other => IrohError::msg(other),
    }
}

#[uniffi::export]
impl IrohConnection {
    /// Open a fresh bidirectional stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn open_bi(&self) -> Result<Arc<IrohStream>, IrohError> {
        let (send, recv) = self.inner.open_bi().await.map_err(map_conn_err)?;
        Ok(Arc::new(IrohStream::new(self.inner.clone(), send, recv)))
    }

    /// Accept the peer's next bidirectional stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn accept_bi(&self) -> Result<Arc<IrohStream>, IrohError> {
        let (send, recv) = self.inner.accept_bi().await.map_err(map_conn_err)?;
        Ok(Arc::new(IrohStream::new(self.inner.clone(), send, recv)))
    }

    /// Open a fresh unidirectional (send-only) stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn open_uni(&self) -> Result<Arc<IrohSendStream>, IrohError> {
        let send = self.inner.open_uni().await.map_err(map_conn_err)?;
        Ok(Arc::new(IrohSendStream::new(self.inner.clone(), send)))
    }

    /// Accept the peer's next unidirectional (recv-only) stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn accept_uni(&self) -> Result<Arc<IrohRecvStream>, IrohError> {
        let recv = self.inner.accept_uni().await.map_err(map_conn_err)?;
        Ok(Arc::new(IrohRecvStream::new(self.inner.clone(), recv)))
    }

    /// Send an unreliable, unordered datagram, waiting for buffer space under congestion.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn send_datagram(&self, data: Vec<u8>) -> Result<(), IrohError> {
        self.inner.send_datagram_wait(data.into()).await.map_err(IrohError::msg)
    }

    /// Send a datagram without waiting for buffer space; fails immediately under congestion.
    pub fn try_send_datagram(&self, data: Vec<u8>) -> Result<(), IrohError> {
        self.inner.send_datagram(data.into()).map_err(IrohError::msg)
    }

    /// Receive the next application datagram.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read_datagram(&self) -> Result<Vec<u8>, IrohError> {
        let bytes = self.inner.read_datagram().await.map_err(map_conn_err)?;
        Ok(bytes.to_vec())
    }

    /// Maximum datagram size accepted by [`send_datagram`](Self::send_datagram); `None` if
    /// datagrams are unsupported by the peer or disabled locally.
    pub fn max_datagram_size(&self) -> Option<u64> {
        self.inner.max_datagram_size().map(|n| n as u64)
    }

    /// Close the connection, notifying the peer with `error_code`/`reason`. Named `shutdown`
    /// rather than `close` to avoid colliding with UniFFI's generated `AutoCloseable.close()`.
    pub fn shutdown(&self, error_code: u64, reason: Vec<u8>) -> Result<(), IrohError> {
        let code = VarInt::from_u64(error_code).map_err(IrohError::msg)?;
        self.inner.close(code, &reason);
        Ok(())
    }

    /// Wait for the connection to close for any reason, returning why.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn closed(&self) -> String {
        self.inner.closed().await.to_string()
    }

    /// The reason the connection closed, if it already has.
    pub fn close_reason(&self) -> Option<String> {
        self.inner.close_reason().map(|e| e.to_string())
    }

    /// Hex endpoint id of the peer.
    pub fn remote_id(&self) -> String {
        self.inner.remote_id().to_string()
    }

    /// The negotiated ALPN, lossily decoded as UTF-8 (same as `accept_next`'s `IncomingConn`).
    pub fn alpn(&self) -> String {
        String::from_utf8_lossy(self.inner.alpn()).into_owned()
    }

    /// A locally-unique, stable identifier for this connection.
    pub fn stable_id(&self) -> u64 {
        self.inner.stable_id() as u64
    }

    /// Current best estimate of the round-trip time on the selected path, in milliseconds.
    /// `None` before a path is established. Same logic as `IrohStream::rtt_ms`.
    pub fn rtt_ms(&self) -> Option<u64> {
        self.inner
            .paths()
            .iter()
            .find(|p| p.is_selected())
            .map(|p| p.rtt().as_millis() as u64)
    }

    /// A snapshot of all network paths currently open for this connection.
    pub fn paths(&self) -> Vec<ConnPath> {
        self.inner
            .paths()
            .iter()
            .map(|p| {
                let kind = if p.is_relay() { PathKind::Relay } else { PathKind::Ip };
                ConnPath {
                    addr: p.remote_addr().to_string(),
                    kind,
                    is_selected: p.is_selected(),
                    rtt_ms: Some(p.rtt().as_millis() as u64),
                }
            })
            .collect()
    }

    /// `"direct"`, `"relay"`, `"mixed"`, or `"none"`, derived from the currently open paths.
    pub fn conn_type(&self) -> String {
        let paths = self.inner.paths();
        let ty = if paths.is_empty() {
            "none"
        } else if let Some(selected) = paths.iter().find(|p| p.is_selected()) {
            if selected.is_relay() { "relay" } else { "direct" }
        } else {
            match (paths.iter().any(|p| p.is_ip()), paths.iter().any(|p| p.is_relay())) {
                (true, true) => "mixed",
                (true, false) => "direct",
                (false, true) => "relay",
                (false, false) => "none",
            }
        };
        ty.into()
    }
}
