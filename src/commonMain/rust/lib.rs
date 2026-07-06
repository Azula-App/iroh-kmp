//! `iroh-kmp` — a UniFFI wrapper around [`iroh`] for Kotlin Multiplatform.
//!
//! It exposes the core `iroh` surface: bind and configure an [`IrohEndpoint`]
//! ([`bind`](endpoint::IrohEndpoint::bind) /
//! [`bind_with`](endpoint::IrohEndpoint::bind_with) + [`EndpointOptions`]), dial
//! peers by ticket, [`NodeAddr`], or node id, accept inbound connections, and work
//! with a first-class [`IrohConnection`] (multiple bi/uni streams, datagrams,
//! close codes, paths/RTT/conn-type), plus address/relay watchers and remote-peer
//! info. The narrow ticket + bidirectional-stream convenience path the azula app's
//! `IrohTransport` uses ([`connect`](endpoint::IrohEndpoint::connect) /
//! [`accept_next`](endpoint::IrohEndpoint::accept_next) / [`IrohStream`]) is kept
//! intact on top of it. It is modeled on `n0-computer/iroh-ffi` but uses a
//! JNI-backed Gobley binding, whose async calls — unlike the published JNA
//! artifact — complete on Android.

mod config;
mod connection;
mod endpoint;
mod error;
mod node_addr;
mod remote_info;
mod stream;

#[cfg(target_os = "android")]
mod android_init;

pub use config::{EndpointOptions, RelayModeOption};
pub use connection::{ConnPath, IrohConnection, PathKind};
pub use endpoint::{IncomingConn, IrohEndpoint};
pub use error::IrohError;
pub use node_addr::{node_addr_from_ticket, ticket_from_node_addr, NodeAddr};
pub use remote_info::{RemoteAddrInfo, RemoteInfo};
pub use stream::{IrohRecvStream, IrohSendStream, IrohStream};

uniffi::setup_scaffolding!();
