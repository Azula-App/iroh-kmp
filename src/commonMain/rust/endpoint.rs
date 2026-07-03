use std::str::FromStr;
use std::sync::Arc;

use iroh::endpoint::presets::{self, Preset as _};
use iroh::{Endpoint, PublicKey, SecretKey, Signature};
use iroh_tickets::endpoint::EndpointTicket;
use iroh_tickets::Ticket as _;

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
        Ok(Arc::new(IrohStream::new(conn, send, recv)))
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
            stream: Arc::new(IrohStream::new(conn, send, recv)),
        }))
    }

    /// Shut the endpoint down, closing all connections.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn shutdown(&self) {
        self.inner.close().await;
    }

    /// Ed25519-sign `data` with this endpoint's secret key, returning the raw
    /// 64-byte signature. Used to sign the invitations payload (see azula-docs
    /// `invitations.md`); verified by peers with [`verify_signature`].
    pub fn sign(&self, data: Vec<u8>) -> Vec<u8> {
        self.inner.secret_key().sign(&data).to_bytes().to_vec()
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

/// Verify an Ed25519 `signature` over `data` against the node id `node_id_hex`.
/// Used to check an invitations payload's signature (see azula-docs
/// `invitations.md`) against the node id embedded in its ticket. Never throws:
/// a malformed node id or signature is just an invalid signature (`false`).
#[uniffi::export]
pub fn verify_signature(node_id_hex: String, data: Vec<u8>, signature: Vec<u8>) -> bool {
    let Ok(key) = PublicKey::from_str(&node_id_hex) else {
        return false;
    };
    let Ok(sig) = Signature::try_from(signature.as_slice()) else {
        return false;
    };
    key.verify(&data, &sig).is_ok()
}

/// Raw (postcard) bytes of an `EndpointTicket` string, for embedding in the
/// invitations payload's `ticket` field (see azula-docs `invitations.md`).
#[uniffi::export]
pub fn ticket_bytes(ticket: String) -> Result<Vec<u8>, IrohError> {
    let t = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
    Ok(t.encode_bytes())
}

/// Reconstruct an `EndpointTicket` string from its raw bytes, the inverse of
/// [`ticket_bytes`].
#[uniffi::export]
pub fn ticket_from_bytes(bytes: Vec<u8>) -> Result<String, IrohError> {
    let t = EndpointTicket::decode_bytes(&bytes).map_err(IrohError::msg)?;
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_verify_roundtrip() {
        let secret_key = vec![7u8; 32];
        let endpoint = IrohEndpoint::bind(vec!["test/sign".into()], Some(secret_key))
            .await
            .expect("bind");
        let node_id = endpoint.node_id();
        let data = b"hello invitations".to_vec();

        let sig = endpoint.sign(data.clone());
        assert_eq!(sig.len(), 64);
        assert!(verify_signature(node_id.clone(), data.clone(), sig.clone()));

        let mut bad_sig = sig.clone();
        bad_sig[0] ^= 0x01;
        assert!(!verify_signature(node_id, data, bad_sig));

        endpoint.shutdown().await;
    }

    #[test]
    fn ticket_bytes_roundtrip() {
        let key = SecretKey::from_bytes(&[3u8; 32]).public();
        let addr = iroh::EndpointAddr::from_parts(key, []);
        let ticket = EndpointTicket::new(addr).to_string();

        let bytes = ticket_bytes(ticket.clone()).expect("ticket_bytes");
        let roundtripped = ticket_from_bytes(bytes).expect("ticket_from_bytes");
        assert_eq!(roundtripped, ticket);
    }

    /// RFC 8032 §7.1 TEST 1 (test-only key, never used for real identity — see
    /// azula-docs `invitations.md`).
    #[test]
    fn rfc8032_test1() {
        let seed = hex_decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let seed: [u8; 32] = seed.try_into().unwrap();
        let expected_pub =
            hex_decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let expected_sig = hex_decode(concat!(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901",
            "555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ));

        let secret = SecretKey::from_bytes(&seed);
        assert_eq!(secret.public().as_bytes().as_slice(), expected_pub.as_slice());

        let sig = secret.sign(b"");
        assert_eq!(sig.to_bytes().to_vec(), expected_sig);

        let node_id_hex = secret.public().to_string();
        assert!(verify_signature(node_id_hex, Vec::new(), expected_sig));
    }
}
