use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::Mutex;

use crate::error::IrohError;

/// A bidirectional QUIC stream. Newline framing stays on the Kotlin side
/// (`LineBuffer`); this exposes only raw byte send/recv.
///
/// The originating [`Connection`] is retained (it's a cheap `Clone` handle) so
/// the stream can surface the live QUIC path RTT via [`rtt_ms`](Self::rtt_ms);
/// it also keeps the connection alive for the stream's lifetime.
#[derive(uniffi::Object)]
pub struct IrohStream {
    conn: Connection,
    send: Arc<Mutex<SendStream>>,
    recv: Arc<Mutex<RecvStream>>,
}

impl IrohStream {
    pub(crate) fn new(conn: Connection, send: SendStream, recv: RecvStream) -> Self {
        IrohStream {
            conn,
            send: Arc::new(Mutex::new(send)),
            recv: Arc::new(Mutex::new(recv)),
        }
    }
}

#[uniffi::export]
impl IrohStream {
    /// Write all of `data` to the stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn send_bytes(&self, data: Vec<u8>) -> Result<(), IrohError> {
        let mut send = self.send.lock().await;
        send.write_all(&data).await.map_err(IrohError::msg)?;
        Ok(())
    }

    /// Current best estimate of this connection's round-trip time, in
    /// milliseconds. Reads iroh's smoothed QUIC RTT for the selected path;
    /// returns `None` before a path is established. Cheap, synchronous snapshot.
    pub fn rtt_ms(&self) -> Option<u64> {
        let paths = self.conn.paths();
        paths
            .iter()
            .find(|p| p.is_selected())
            .map(|p| p.rtt().as_millis() as u64)
    }

    /// Read up to `max` bytes. Returns `None` at end-of-stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn recv(&self, max: u32) -> Result<Option<Vec<u8>>, IrohError> {
        let mut buf = vec![0u8; max as usize];
        let mut recv = self.recv.lock().await;
        match recv.read(&mut buf).await.map_err(IrohError::msg)? {
            Some(n) => {
                buf.truncate(n);
                Ok(Some(buf))
            }
            None => Ok(None),
        }
    }

    /// Signal that no more data will be sent (finishes the send half). Named
    /// `finish` rather than `close` to avoid colliding with UniFFI's
    /// `Disposable.close()` on the generated object.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn finish(&self) -> Result<(), IrohError> {
        let mut send = self.send.lock().await;
        send.finish().map_err(IrohError::msg)?;
        Ok(())
    }
}
