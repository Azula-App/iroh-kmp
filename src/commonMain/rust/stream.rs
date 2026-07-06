use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
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

    /// Read exactly `n` bytes, or error at end-of-stream before that many arrive.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read_exact(&self, n: u32) -> Result<Vec<u8>, IrohError> {
        let mut buf = vec![0u8; n as usize];
        let mut recv = self.recv.lock().await;
        recv.read_exact(&mut buf).await.map_err(IrohError::msg)?;
        Ok(buf)
    }

    /// Read until end-of-stream, erroring if more than `max` bytes arrive.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read_to_end(&self, max: u32) -> Result<Vec<u8>, IrohError> {
        let mut recv = self.recv.lock().await;
        recv.read_to_end(max as usize).await.map_err(IrohError::msg)
    }

    /// Set the send half's transmission priority (higher sends first).
    pub fn set_priority(&self, priority: i32) -> Result<(), IrohError> {
        let send = self.send.blocking_lock();
        send.set_priority(priority).map_err(IrohError::msg)
    }

    /// The send half's current transmission priority.
    pub fn priority(&self) -> Result<i32, IrohError> {
        let send = self.send.blocking_lock();
        send.priority().map_err(IrohError::msg)
    }

    /// Abruptly reset the send half, telling the peer to stop reading.
    pub fn reset(&self, error_code: u64) -> Result<(), IrohError> {
        let code = VarInt::from_u64(error_code).map_err(IrohError::msg)?;
        let mut send = self.send.blocking_lock();
        send.reset(code).map_err(IrohError::msg)
    }

    /// Tell the peer to stop sending on the recv half.
    pub fn stop(&self, error_code: u64) -> Result<(), IrohError> {
        let code = VarInt::from_u64(error_code).map_err(IrohError::msg)?;
        let mut recv = self.recv.blocking_lock();
        recv.stop(code).map_err(IrohError::msg)
    }

    /// The send half's stream id.
    pub fn send_id(&self) -> u64 {
        u64::from(self.send.blocking_lock().id())
    }

    /// The recv half's stream id.
    pub fn recv_id(&self) -> u64 {
        u64::from(self.recv.blocking_lock().id())
    }
}

/// A unidirectional, send-only QUIC stream, opened via [`crate::IrohConnection::open_uni`].
///
/// Retains the originating [`Connection`] for the same reason [`IrohStream`] does.
#[derive(uniffi::Object)]
pub struct IrohSendStream {
    #[allow(dead_code)] // kept alive for the stream's lifetime; not otherwise read yet
    conn: Connection,
    inner: Arc<Mutex<SendStream>>,
}

impl IrohSendStream {
    pub(crate) fn new(conn: Connection, send: SendStream) -> Self {
        IrohSendStream { conn, inner: Arc::new(Mutex::new(send)) }
    }
}

#[uniffi::export]
impl IrohSendStream {
    /// Write all of `data` to the stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn send_bytes(&self, data: Vec<u8>) -> Result<(), IrohError> {
        let mut send = self.inner.lock().await;
        send.write_all(&data).await.map_err(IrohError::msg)
    }

    /// Signal that no more data will be sent.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn finish(&self) -> Result<(), IrohError> {
        let mut send = self.inner.lock().await;
        send.finish().map_err(IrohError::msg)
    }

    /// Abruptly reset the stream, telling the peer to stop reading.
    pub fn reset(&self, error_code: u64) -> Result<(), IrohError> {
        let code = VarInt::from_u64(error_code).map_err(IrohError::msg)?;
        let mut send = self.inner.blocking_lock();
        send.reset(code).map_err(IrohError::msg)
    }

    /// Set this stream's transmission priority (higher sends first).
    pub fn set_priority(&self, priority: i32) -> Result<(), IrohError> {
        let send = self.inner.blocking_lock();
        send.set_priority(priority).map_err(IrohError::msg)
    }

    /// This stream's current transmission priority.
    pub fn priority(&self) -> Result<i32, IrohError> {
        let send = self.inner.blocking_lock();
        send.priority().map_err(IrohError::msg)
    }

    /// Resolves once the peer stops the stream (returning its error code) or acknowledges
    /// a `finish()`ed stream in full (`None`).
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn stopped(&self) -> Result<Option<u64>, IrohError> {
        let stopped = self.inner.lock().await.stopped();
        stopped.await.map(|code| code.map(u64::from)).map_err(IrohError::msg)
    }

    /// This stream's id.
    pub fn id(&self) -> u64 {
        u64::from(self.inner.blocking_lock().id())
    }
}

/// A unidirectional, recv-only QUIC stream, opened via [`crate::IrohConnection::accept_uni`].
///
/// Retains the originating [`Connection`] for the same reason [`IrohStream`] does.
#[derive(uniffi::Object)]
pub struct IrohRecvStream {
    #[allow(dead_code)] // kept alive for the stream's lifetime; not otherwise read yet
    conn: Connection,
    inner: Arc<Mutex<RecvStream>>,
}

impl IrohRecvStream {
    pub(crate) fn new(conn: Connection, recv: RecvStream) -> Self {
        IrohRecvStream { conn, inner: Arc::new(Mutex::new(recv)) }
    }
}

#[uniffi::export]
impl IrohRecvStream {
    /// Read up to `max` bytes. Returns `None` at end-of-stream.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn recv(&self, max: u32) -> Result<Option<Vec<u8>>, IrohError> {
        let mut buf = vec![0u8; max as usize];
        let mut recv = self.inner.lock().await;
        match recv.read(&mut buf).await.map_err(IrohError::msg)? {
            Some(n) => {
                buf.truncate(n);
                Ok(Some(buf))
            }
            None => Ok(None),
        }
    }

    /// Read exactly `n` bytes, or error at end-of-stream before that many arrive.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read_exact(&self, n: u32) -> Result<Vec<u8>, IrohError> {
        let mut buf = vec![0u8; n as usize];
        let mut recv = self.inner.lock().await;
        recv.read_exact(&mut buf).await.map_err(IrohError::msg)?;
        Ok(buf)
    }

    /// Read until end-of-stream, erroring if more than `max` bytes arrive.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read_to_end(&self, max: u32) -> Result<Vec<u8>, IrohError> {
        let mut recv = self.inner.lock().await;
        recv.read_to_end(max as usize).await.map_err(IrohError::msg)
    }

    /// Tell the peer to stop sending on this stream.
    pub fn stop(&self, error_code: u64) -> Result<(), IrohError> {
        let code = VarInt::from_u64(error_code).map_err(IrohError::msg)?;
        let mut recv = self.inner.blocking_lock();
        recv.stop(code).map_err(IrohError::msg)
    }

    /// This stream's id.
    pub fn id(&self) -> u64 {
        u64::from(self.inner.blocking_lock().id())
    }
}
