use std::str::FromStr;
use std::sync::Arc;

use iroh::endpoint::presets::{self, Preset as _};
use iroh::{Endpoint, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;

use crate::error::IrohError;
use crate::stream::IrohStream;

/// An accepted inbound connection: who dialed us, on which ALPN, and the
/// already-accepted bidirectional stream to talk over.
#[derive(uniffi::Record)]
pub struct IncomingConn {
    pub remote_id: String,
    pub alpn: String,
    pub stream: Arc<IrohStream>,
}

/// A bound iroh endpoint. Mirrors the app's `IrohTransport`:
/// [`bind`](Self::bind) → [`my_ticket`](Self::my_ticket) / [`connect`](Self::connect)
/// / [`accept_next`](Self::accept_next).
#[derive(uniffi::Object)]
pub struct IrohEndpoint {
    inner: Endpoint,
}

#[uniffi::export]
impl IrohEndpoint {
    /// Bind an endpoint advertising `alpns`, reusing `secret_key` (32 bytes) for
    /// a stable node id when provided. Uses the n0 production preset (relays +
    /// discovery). On Android, `IrohAndroid.installAndroidContext` must have been
    /// called first (iroh's DNS resolver needs the JavaVM + app context).
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn bind(
        alpns: Vec<String>,
        secret_key: Option<Vec<u8>>,
    ) -> Result<Arc<Self>, IrohError> {
        // An empty builder + a preset installs the crypto provider; layering the
        // explicit options on top mirrors iroh-ffi's `Endpoint::bind`.
        let mut builder = presets::N0.apply(iroh::endpoint::Builder::empty());
        if let Some(bytes) = secret_key {
            let key: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| IrohError::Generic { msg: "secret key must be 32 bytes".into() })?;
            builder = builder.secret_key(SecretKey::from_bytes(&key));
        }
        let alpns: Vec<Vec<u8>> = alpns.into_iter().map(String::into_bytes).collect();
        builder = builder.alpns(alpns);

        let inner = builder.bind().await.map_err(IrohError::msg)?;

        // Bring discovery/relays up in the background so we're reachable without
        // waiting for the first `my_ticket` call (mirrors the old transport).
        let warm = inner.clone();
        tokio::spawn(async move {
            warm.online().await;
        });

        Ok(Arc::new(IrohEndpoint { inner }))
    }

    /// Hex node id of this endpoint (stable across binds for a given secret key).
    pub fn node_id(&self) -> String {
        self.inner.id().to_string()
    }

    /// The 32-byte secret key, so the caller can persist it and keep a stable id.
    pub fn secret_key_bytes(&self) -> Vec<u8> {
        self.inner.secret_key().to_bytes().to_vec()
    }

    /// Wait until a home relay is available, then return a shareable ticket
    /// encoding our address (the user's "code").
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn my_ticket(&self) -> Result<String, IrohError> {
        self.inner.online().await;
        Ok(EndpointTicket::new(self.inner.addr()).to_string())
    }

    /// Dial a peer by `ticket` on `alpn`, opening a fresh bidirectional stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn connect(&self, ticket: String, alpn: String) -> Result<Arc<IrohStream>, IrohError> {
        let ticket = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
        let addr = ticket.endpoint_addr().clone();
        let conn = self
            .inner
            .connect(addr, alpn.as_bytes())
            .await
            .map_err(IrohError::msg)?;
        let (send, recv) = conn.open_bi().await.map_err(IrohError::msg)?;
        Ok(Arc::new(IrohStream::new(send, recv)))
    }

    /// Block until the next inbound connection completes its handshake and opens
    /// a bidirectional stream. Returns `None` once the endpoint is closed; the
    /// Kotlin side loops this into a `Flow<IncomingConnection>`.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn accept_next(&self) -> Result<Option<IncomingConn>, IrohError> {
        let incoming = match self.inner.accept().await {
            Some(incoming) => incoming,
            None => return Ok(None),
        };
        let accepting = incoming.accept().map_err(IrohError::msg)?;
        let conn = accepting.await.map_err(|e| IrohError::Generic { msg: format!("{e:?}") })?;
        let remote_id = conn.remote_id().to_string();
        let alpn = String::from_utf8_lossy(&conn.alpn().to_vec()).into_owned();
        let (send, recv) = conn.accept_bi().await.map_err(IrohError::msg)?;
        Ok(Some(IncomingConn {
            remote_id,
            alpn,
            stream: Arc::new(IrohStream::new(send, recv)),
        }))
    }

    /// Shut the endpoint down, closing all connections.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn shutdown(&self) {
        self.inner.close().await;
    }
}

/// Decode the peer's hex node id from a shareable ticket, without dialing. Lets
/// the app key/name a conversation after the peer it's connecting out to (inbound
/// connections already expose `remote_id`).
#[uniffi::export]
pub fn node_id_from_ticket(ticket: String) -> Result<String, IrohError> {
    let t = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
    Ok(t.endpoint_addr().id.to_string())
}
