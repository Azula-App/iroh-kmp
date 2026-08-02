use std::net::SocketAddr;
use std::str::FromStr;

use iroh::{PublicKey, RelayUrl, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;

use crate::error::IrohError;

/// A snapshot of an endpoint's network addresses: its id, home relay, and known direct (IP)
/// addresses. Structured equivalent of a ticket string, for apps that already have their own
/// channel for exchanging addresses (e.g. over an established stream) rather than a
/// user-facing "code".
///
/// The flattened counterpart of [`iroh::EndpointAddr`], whose `addrs` set mixes relay and IP
/// transports in one collection; UniFFI has no sum type that can carry that across the FFI,
/// so the relay and direct addresses are split into separate fields here.
#[derive(uniffi::Record, Clone)]
pub struct EndpointAddr {
    pub id: String,
    pub relay_url: Option<String>,
    pub direct_addresses: Vec<String>,
}

impl TryFrom<EndpointAddr> for iroh::EndpointAddr {
    type Error = IrohError;

    fn try_from(addr: EndpointAddr) -> Result<Self, Self::Error> {
        let id = PublicKey::from_str(&addr.id).map_err(IrohError::msg)?;

        let mut addrs = Vec::with_capacity(addr.direct_addresses.len() + 1);
        for ip in &addr.direct_addresses {
            let sock: SocketAddr = ip.parse().map_err(IrohError::msg)?;
            addrs.push(TransportAddr::Ip(sock));
        }
        if let Some(relay) = &addr.relay_url {
            let url = RelayUrl::from_str(relay).map_err(IrohError::msg)?;
            addrs.push(TransportAddr::Relay(url));
        }

        Ok(iroh::EndpointAddr::from_parts(id, addrs))
    }
}

impl From<iroh::EndpointAddr> for EndpointAddr {
    fn from(addr: iroh::EndpointAddr) -> Self {
        EndpointAddr {
            id: addr.id.to_string(),
            relay_url: addr.relay_urls().next().map(ToString::to_string),
            direct_addresses: addr.ip_addrs().map(ToString::to_string).collect(),
        }
    }
}

/// Decode the peer's structured [`EndpointAddr`] from a shareable ticket, without dialing. The
/// structured counterpart of [`crate::endpoint::endpoint_id_from_ticket`], for apps that want
/// to hand the addresses to their own transport rather than re-share the opaque ticket string.
#[uniffi::export]
pub fn endpoint_addr_from_ticket(ticket: String) -> Result<EndpointAddr, IrohError> {
    let t = EndpointTicket::from_str(&ticket).map_err(IrohError::msg)?;
    Ok(t.endpoint_addr().clone().into())
}

/// Re-encode an [`EndpointAddr`] as a shareable ticket string, the inverse of
/// [`endpoint_addr_from_ticket`].
#[uniffi::export]
pub fn ticket_from_endpoint_addr(addr: EndpointAddr) -> Result<String, IrohError> {
    let addr: iroh::EndpointAddr = addr.try_into()?;
    Ok(EndpointTicket::new(addr).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_addr_iroh_roundtrip() {
        let id = iroh::SecretKey::from_bytes(&[9u8; 32]).public().to_string();
        let addr = EndpointAddr {
            id: id.clone(),
            relay_url: Some("https://relay.example.com./".into()),
            direct_addresses: vec!["127.0.0.1:1234".into(), "[::1]:5678".into()],
        };

        let iroh_addr: iroh::EndpointAddr = addr.clone().try_into().expect("try_into");
        assert_eq!(iroh_addr.id.to_string(), id);
        assert_eq!(iroh_addr.relay_urls().count(), 1);
        assert_eq!(iroh_addr.ip_addrs().count(), 2);

        let roundtripped: EndpointAddr = iroh_addr.into();
        assert_eq!(roundtripped.id, addr.id);
        assert_eq!(roundtripped.relay_url, addr.relay_url);

        let mut expected = addr.direct_addresses;
        expected.sort();
        let mut got = roundtripped.direct_addresses;
        got.sort();
        assert_eq!(got, expected);
    }
}
