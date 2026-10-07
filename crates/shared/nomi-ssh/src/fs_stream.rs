//! Bound SFTP frames before the dependency allocates their payload, and make
//! session cancellation interrupt both halves even during a blocked write.
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{StreamExt, TryStreamExt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::{
    codec::LengthDelimitedCodec,
    io::StreamReader,
    sync::{CancellationToken, DropGuard},
};

/// The dependency advertises this default but does not apply it on reads.
const MAX_PACKET_BYTES: usize = 256 * 1024;

pub(super) fn bounded_stream<S>(
    stream: S,
) -> (impl AsyncRead + AsyncWrite + Unpin + Send, DropGuard)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    let cancel = CancellationToken::new();
    let frames = LengthDelimitedCodec::builder()
        .length_field_length(4)
        .length_adjustment(4)
        .num_skip(0)
        .max_frame_length(MAX_PACKET_BYTES)
        .new_read(read)
        .map_ok(|frame| frame.freeze())
        .take_until(cancel.clone().cancelled_owned())
        .boxed();
    let stream = SessionStream {
        read: Box::pin(StreamReader::new(frames)),
        write,
        cancelled: Box::pin(cancel.clone().cancelled_owned()),
    };
    (stream, cancel.drop_guard())
}

struct SessionStream<W> {
    read: Pin<Box<dyn AsyncRead + Send>>,
    write: W,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl<W: Unpin> AsyncRead for SessionStream<W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.read.as_mut().poll_read(cx, buf)
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for SessionStream<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "SFTP session retired",
            )));
        }
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn framing_preserves_partial_headers_and_the_exact_limit_payload() {
        let (client, mut peer) = tokio::io::duplex(1024);
        let (mut stream, _guard) = bounded_stream(client);
        let mut wire = (MAX_PACKET_BYTES as u32).to_be_bytes().to_vec();
        wire.extend(vec![42; MAX_PACKET_BYTES]);
        wire.extend_from_slice(&[0, 0, 0, 1, 99]);
        let mut received = vec![0; wire.len()];
        let send = async {
            peer.write_all(&wire[..2]).await.unwrap();
            tokio::task::yield_now().await;
            peer.write_all(&wire[2..]).await.unwrap();
        };
        let receive = async {
            stream.read_exact(&mut received).await.unwrap();
        };
        tokio::join!(send, receive);
        assert_eq!(received, wire);
    }

    #[tokio::test]
    async fn frame_limit_rejects_the_first_byte_over_the_boundary() {
        let (client, mut peer) = tokio::io::duplex(16);
        let (mut stream, _guard) = bounded_stream(client);
        peer.write_u32(MAX_PACKET_BYTES as u32 + 1).await.unwrap();
        let error = stream.read_u32().await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(stream.read(&mut [0; 1]).await.unwrap(), 0);
    }
}
