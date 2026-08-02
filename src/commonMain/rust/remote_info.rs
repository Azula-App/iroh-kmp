use crate::connection::PathKind;

/// A single known transport address for a remote endpoint, and whether it's currently in use.
#[derive(uniffi::Record)]
pub struct RemoteAddrInfo {
    pub addr: String,
    pub kind: PathKind,
    pub usage: String,
}

/// A snapshot of everything this endpoint currently knows about a remote peer's addressing.
/// Returned by [`crate::IrohEndpoint::remote_info`].
#[derive(uniffi::Record)]
pub struct RemoteInfo {
    pub id: String,
    pub addrs: Vec<RemoteAddrInfo>,
}
