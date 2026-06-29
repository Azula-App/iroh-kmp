use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::Mutex;

use crate::error::IrohError;

/// A bidirectional QUIC stream. Newline framing stays on the Kotlin side
/// (`LineBuffer`); this exposes only raw byte send/recv.
#[derive(uniffi::Object)]
pub struct IrohStream {
    send: Arc<Mutex<SendStream>>,
    recv: Arc<Mutex<RecvStream>>,
}

impl IrohStream {
    pub(crate) fn new(send: SendStream, recv: RecvStream) -> Self {
        IrohStream {
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
