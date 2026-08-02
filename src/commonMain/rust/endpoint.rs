use std::str::FromStr;
use std::sync::Arc;

use iroh::{Endpoint, PublicKey, Signature, Watcher as _};
#[cfg(test)]
use iroh::SecretKey;
use iroh_tickets::endpoint::EndpointTicket;
use iroh_tickets::Ticket as _;

use crate::config::EndpointOptions;
use crate::connection::{IrohConnection, PathKind};
use crate::error::IrohError;
use crate::endpoint_addr::EndpointAddr;
use crate::remote_info::{RemoteAddrInfo, RemoteInfo};
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
    ///
    /// A thin delegate to [`Self::bind_with`] with every option at its default; kept
    /// separate because it's the common case and predates [`EndpointOptions`].
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn bind(
        alpns: Vec<String>,
        secret_key: Option<Vec<u8>>,
    ) -> Result<Arc<Self>, IrohError> {
        Self::bind_with(EndpointOptions {
            alpns,
            secret_key,
            relay_mode: None,
            address_lookup: true,
            bind_addr: None,
            external_addrs: None,
            warm_up_online: true,
        })
        .await
    }

    /// Bind an endpoint from fully-specified [`EndpointOptions`]. See [`Self::bind`] for the
    /// common case (n0 relays + discovery, no explicit bind address).
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn bind_with(options: EndpointOptions) -> Result<Arc<Self>, IrohError> {
        let warm_up_online = options.warm_up_online;
        let builder = crate::config::builder_from_options(options)?;
        let inner = builder.bind().await.map_err(IrohError::msg)?;

        if warm_up_online {
            // Bring discovery/relays up in the background so we're reachable without
            // waiting for the first `my_ticket` call (mirrors the old transport).
            let warm = inner.clone();
            tokio::spawn(async move {
                warm.online().await;
            });
        }

        Ok(Arc::new(IrohEndpoint { inner }))
    }

    /// Hex endpoint id of this endpoint (stable across binds for a given secret key).
    pub fn id(&self) -> String {
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
        let conn = self.connect_conn(ticket, alpn).await?;
        conn.open_bi().await
    }

    /// Dial a peer by `ticket` on `alpn`, returning the established connection without
    /// opening a stream. See [`Self::connect_addr`] and [`Self::connect_by_id`] for
    /// dialing without a ticket, and [`Self::connect`] for the ticket+bi-stream convenience.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn connect_conn(&self, ticket: String, alpn: String) -> Result<Arc<IrohConnection>, IrohError> {
        let ticket = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
        let addr = ticket.endpoint_addr().clone();
        let conn = self.inner.connect(addr, alpn.as_bytes()).await.map_err(IrohError::msg)?;
        Ok(Arc::new(IrohConnection::new(conn)))
    }

    /// Dial a peer by its structured [`EndpointAddr`] on `alpn`.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn connect_addr(&self, addr: EndpointAddr, alpn: String) -> Result<Arc<IrohConnection>, IrohError> {
        let addr: iroh::EndpointAddr = addr.try_into()?;
        let conn = self.inner.connect(addr, alpn.as_bytes()).await.map_err(IrohError::msg)?;
        Ok(Arc::new(IrohConnection::new(conn)))
    }

    /// Dial a peer by hex endpoint id alone on `alpn`, relying on the endpoint's address
    /// lookup service to resolve reachable addresses.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn connect_by_id(&self, endpoint_id_hex: String, alpn: String) -> Result<Arc<IrohConnection>, IrohError> {
        let id = PublicKey::from_str(&endpoint_id_hex).map_err(IrohError::msg)?;
        let conn = self.inner.connect(id, alpn.as_bytes()).await.map_err(IrohError::msg)?;
        Ok(Arc::new(IrohConnection::new(conn)))
    }

    /// Block until the next inbound connection completes its handshake and opens
    /// a bidirectional stream. Returns `None` once the endpoint is closed; the
    /// Kotlin side loops this into a `Flow<IncomingConnection>`.
    ///
    /// Shares the accept queue with [`Self::accept_conn`] — both are single-consumer, so
    /// don't call them concurrently expecting each to see every incoming connection.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn accept_next(&self) -> Result<Option<IncomingConn>, IrohError> {
        let conn = match self.accept_conn().await? {
            Some(conn) => conn,
            None => return Ok(None),
        };
        let remote_id = conn.remote_id();
        let alpn = conn.alpn();
        let stream = conn.accept_bi().await?;
        Ok(Some(IncomingConn { remote_id, alpn, stream }))
    }

    /// Block until the next inbound connection completes its handshake. Returns `None`
    /// once the endpoint is closed. See [`Self::accept_next`]'s note on the shared queue.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn accept_conn(&self) -> Result<Option<Arc<IrohConnection>>, IrohError> {
        let incoming = match self.inner.accept().await {
            Some(incoming) => incoming,
            None => return Ok(None),
        };
        let accepting = incoming.accept().map_err(IrohError::msg)?;
        let conn = accepting.await.map_err(|e| IrohError::Generic { msg: format!("{e:?}") })?;
        Ok(Some(Arc::new(IrohConnection::new(conn))))
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

    /// A snapshot of this endpoint's current addressing info (relay + direct addresses).
    /// May be incomplete until [`Self::wait_online`] resolves; see [`Self::addr_updated`]
    /// to wait for a change.
    pub fn addr(&self) -> EndpointAddr {
        self.inner.addr().into()
    }

    /// Waits for this endpoint's addressing info to change, then returns the new snapshot.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn addr_updated(&self) -> EndpointAddr {
        let mut watcher = self.inner.watch_addr();
        match watcher.updated().await {
            Ok(addr) => addr.into(),
            Err(_disconnected) => watcher.get().into(),
        }
    }

    /// This endpoint's known direct (IP) addresses, as `"ip:port"` strings.
    pub fn direct_addresses(&self) -> Vec<String> {
        self.inner.addr().ip_addrs().map(ToString::to_string).collect()
    }

    /// The URL of a currently-connected home relay, if any.
    pub fn home_relay(&self) -> Option<String> {
        let mut watcher = self.inner.home_relay_status();
        watcher.get().into_iter().find(|status| status.is_connected()).map(|status| status.url().to_string())
    }

    /// Wait until this endpoint is considered "online" (connected to at least one relay).
    /// Pends forever if no relays are configured.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn wait_online(&self) {
        self.inner.online().await;
    }

    /// Whether [`Self::shutdown`] has already been called.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// The local socket addresses this endpoint's sockets are bound to.
    pub fn bound_sockets(&self) -> Vec<String> {
        self.inner.bound_sockets().into_iter().map(|a| a.to_string()).collect()
    }

    /// Replace the ALPNs this endpoint accepts on incoming connections.
    pub fn set_alpns(&self, alpns: Vec<String>) {
        let alpns: Vec<Vec<u8>> = alpns.into_iter().map(String::into_bytes).collect();
        self.inner.set_alpns(alpns);
    }

    /// Notify the endpoint of a potential network change (e.g. Android's
    /// `ConnectivityManager` callbacks, which iroh can't observe on its own).
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn network_change(&self) {
        self.inner.network_change().await;
    }

    // TODO: expose endpoint metrics (metrics cargo feature)

    /// Addressing info this endpoint currently knows about `endpoint_id_hex`, or `None` if
    /// it's unknown or the endpoint is closed. A snapshot in time, not a watcher.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn remote_info(&self, endpoint_id_hex: String) -> Result<Option<RemoteInfo>, IrohError> {
        let id = PublicKey::from_str(&endpoint_id_hex).map_err(IrohError::msg)?;
        let Some(info) = self.inner.remote_info(id).await else {
            return Ok(None);
        };
        let id = info.id().to_string();
        let addrs = info
            .into_addrs()
            .map(|a| {
                let usage = match a.usage() {
                    iroh::endpoint::TransportAddrUsage::Active => "active",
                    iroh::endpoint::TransportAddrUsage::Inactive => "inactive",
                    _ => "unknown",
                }
                .to_string();
                let (addr, kind) = match a.into_addr() {
                    iroh::TransportAddr::Ip(sock) => (sock.to_string(), PathKind::Ip),
                    iroh::TransportAddr::Relay(url) => (url.to_string(), PathKind::Relay),
                    other => (other.to_string(), PathKind::Ip),
                };
                RemoteAddrInfo { addr, kind, usage }
            })
            .collect();
        Ok(Some(RemoteInfo { id, addrs }))
    }
}

/// Decode the peer's hex endpoint id from a shareable ticket, without dialing. Lets
/// the app key/name a conversation after the peer it's connecting out to (inbound
/// connections already expose `remote_id`).
#[uniffi::export]
pub fn endpoint_id_from_ticket(ticket: String) -> Result<String, IrohError> {
    let t = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
    Ok(t.endpoint_addr().id.to_string())
}

/// Verify an Ed25519 `signature` over `data` against the endpoint id `endpoint_id_hex`.
/// Used to check an invitations payload's signature (see azula-docs
/// `invitations.md`) against the endpoint id embedded in its ticket. Never throws:
/// a malformed endpoint id or signature is just an invalid signature (`false`).
#[uniffi::export]
pub fn verify_signature(endpoint_id_hex: String, data: Vec<u8>, signature: Vec<u8>) -> bool {
    let Ok(key) = PublicKey::from_str(&endpoint_id_hex) else {
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
    use crate::config::RelayModeOption;

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Binds an offline-friendly endpoint: no relays, no address lookup, alpn `alpn`.
    async fn bind_offline(alpn: &str) -> Arc<IrohEndpoint> {
        IrohEndpoint::bind_with(EndpointOptions {
            alpns: vec![alpn.into()],
            secret_key: None,
            relay_mode: Some(RelayModeOption::Disabled),
            address_lookup: false,
            bind_addr: None,
            external_addrs: None,
            warm_up_online: false,
        })
        .await
        .expect("bind_with")
    }

    /// This endpoint's loopback [`EndpointAddr`]: its bound port reachable via `127.0.0.1`,
    /// sidestepping relay/address-lookup/net-report entirely for a fast, offline test.
    fn loopback_addr(ep: &IrohEndpoint) -> EndpointAddr {
        let port = ep
            .bound_sockets()
            .into_iter()
            .find_map(|s| s.parse::<std::net::SocketAddr>().ok())
            .expect("at least one bound socket")
            .port();
        EndpointAddr {
            id: ep.id(),
            relay_url: None,
            direct_addresses: vec![format!("127.0.0.1:{port}")],
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bind_with_defaults_matches_bind() {
        let secret_key = vec![5u8; 32];
        let via_bind = IrohEndpoint::bind(vec!["test/bind-with".into()], Some(secret_key.clone()))
            .await
            .expect("bind");
        let via_bind_with = IrohEndpoint::bind_with(EndpointOptions {
            alpns: vec!["test/bind-with".into()],
            secret_key: Some(secret_key),
            relay_mode: None,
            address_lookup: true,
            bind_addr: None,
            external_addrs: None,
            warm_up_online: true,
        })
        .await
        .expect("bind_with");

        assert_eq!(via_bind.id(), via_bind_with.id());
        assert_eq!(via_bind.secret_key_bytes(), via_bind_with.secret_key_bytes());
        assert!(!via_bind.is_closed());
        assert!(!via_bind_with.is_closed());

        via_bind.shutdown().await;
        via_bind_with.shutdown().await;
    }

    /// Binds with `address_lookup` on and the given relay mode, returning how many address
    /// lookup services the endpoint ended up with.
    async fn lookup_service_count(relay_mode: Option<RelayModeOption>) -> usize {
        let ep = IrohEndpoint::bind_with(EndpointOptions {
            alpns: vec!["test/lookup".into()],
            secret_key: None,
            relay_mode,
            address_lookup: true,
            bind_addr: None,
            external_addrs: None,
            warm_up_online: false,
        })
        .await
        .expect("bind_with");
        let n = ep.inner.address_lookup().expect("address_lookup").len();
        ep.shutdown().await;
        n
    }

    /// Turning relays off or pointing them elsewhere must not quietly cost us an address
    /// lookup service. `builder_from_options` gets these by subtracting from `presets::N0`
    /// rather than re-listing its contents, because the re-listed version drifted: it kept
    /// publishing via pkarr while resolving over DNS alone once iroh 1.0.3 added
    /// `PkarrResolver` to the preset. Asserting against the default path pins that, and
    /// keeps pinning it as n0 adds services.
    #[tokio::test(flavor = "current_thread")]
    async fn address_lookup_survives_non_default_relay_modes() {
        let default_count = lookup_service_count(None).await;
        assert!(default_count > 0, "the n0 preset should install address lookup services");

        assert_eq!(
            lookup_service_count(Some(RelayModeOption::Disabled)).await,
            default_count,
            "disabling relays dropped an address lookup service",
        );
        assert_eq!(
            lookup_service_count(Some(RelayModeOption::Custom {
                urls: vec!["https://example.invalid".into()],
            }))
            .await,
            default_count,
            "a custom relay dropped an address lookup service",
        );
    }

    /// The other half of the contract: `address_lookup: false` clears every service, so an
    /// offline endpoint really is offline rather than quietly publishing to n0.
    #[tokio::test(flavor = "current_thread")]
    async fn address_lookup_false_clears_every_service() {
        let ep = bind_offline("test/no-lookup").await;
        assert_eq!(ep.inner.address_lookup().expect("address_lookup").len(), 0);
        ep.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connect_addr_bi_uni_and_datagram_roundtrip() {
        let alpn = "test/conn";
        let a = bind_offline(alpn).await;
        let b = bind_offline(alpn).await;
        let b_addr = loopback_addr(&b);

        let accept_task = tokio::spawn({
            let b = b.clone();
            async move { b.accept_conn().await }
        });
        let conn_a = a.connect_addr(b_addr, alpn.into()).await.expect("connect_addr");
        let conn_b = accept_task
            .await
            .expect("join")
            .expect("accept_conn")
            .expect("Some(conn)");

        // Bidirectional stream roundtrip.
        let bi_accept = tokio::spawn({
            let conn_b = conn_b.clone();
            async move { conn_b.accept_bi().await }
        });
        let bi_a = conn_a.open_bi().await.expect("open_bi");
        bi_a.send_bytes(b"hello bi".to_vec()).await.expect("send_bytes");
        bi_a.finish().await.expect("finish");
        let bi_b = bi_accept.await.expect("join").expect("accept_bi");
        assert_eq!(bi_b.read_to_end(1024).await.expect("read_to_end"), b"hello bi");

        // Unidirectional stream roundtrip.
        let uni_accept = tokio::spawn({
            let conn_b = conn_b.clone();
            async move { conn_b.accept_uni().await }
        });
        let uni_send = conn_a.open_uni().await.expect("open_uni");
        uni_send.send_bytes(b"hello uni".to_vec()).await.expect("send_bytes");
        uni_send.finish().await.expect("finish");
        let uni_recv = uni_accept.await.expect("join").expect("accept_uni");
        assert_eq!(uni_recv.read_to_end(1024).await.expect("read_to_end"), b"hello uni");

        // Datagram roundtrip.
        conn_a.send_datagram(b"hello dgram".to_vec()).await.expect("send_datagram");
        assert_eq!(conn_b.read_datagram().await.expect("read_datagram"), b"hello dgram");

        conn_a.shutdown(0, Vec::new()).expect("shutdown conn_a");
        conn_b.shutdown(0, Vec::new()).expect("shutdown conn_b");
        a.shutdown().await;
        b.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_verify_roundtrip() {
        let secret_key = vec![7u8; 32];
        let endpoint = IrohEndpoint::bind(vec!["test/sign".into()], Some(secret_key))
            .await
            .expect("bind");
        let id = endpoint.id();
        let data = b"hello invitations".to_vec();

        let sig = endpoint.sign(data.clone());
        assert_eq!(sig.len(), 64);
        assert!(verify_signature(id.clone(), data.clone(), sig.clone()));

        let mut bad_sig = sig.clone();
        bad_sig[0] ^= 0x01;
        assert!(!verify_signature(id, data, bad_sig));

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

        let endpoint_id_hex = secret.public().to_string();
        assert!(verify_signature(endpoint_id_hex, Vec::new(), expected_sig));
    }
}
