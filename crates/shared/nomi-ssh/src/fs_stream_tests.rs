use super::Session;
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::Notify;

#[derive(Default)]
struct Observation {
    block_writes: AtomicBool,
    write_blocked: Notify,
    block_shutdown: AtomicBool,
    shutdown_blocked: Notify,
    dropped: Notify,
}

struct ObservedStream {
    stream: DuplexStream,
    observation: Arc<Observation>,
}

impl Drop for ObservedStream {
    fn drop(&mut self) {
        self.observation.dropped.notify_one();
    }
}

impl AsyncRead for ObservedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for ObservedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.observation.block_writes.load(Ordering::SeqCst) {
            self.observation.write_blocked.notify_one();
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.observation.block_shutdown.load(Ordering::SeqCst) {
            self.observation.shutdown_blocked.notify_one();
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

async fn connected_stream() -> (Session, DuplexStream, Arc<Observation>) {
    let (client, mut peer) = tokio::io::duplex(32 * 1024);
    let observation = Arc::new(Observation::default());
    let handshake = tokio::spawn(async move {
        let size = peer.read_u32().await.unwrap();
        assert!(size <= 64, "only a small INIT is expected");
        let mut init = vec![0; size as usize];
        peer.read_exact(&mut init).await.unwrap();
        assert_eq!(init[0], 1);
        // SFTP VERSION 3: payload length 5, packet type 2, version 3.
        peer.write_all(&[0, 0, 0, 5, 2, 0, 0, 0, 3]).await.unwrap();
        peer
    });
    let session = Session::new(ObservedStream {
        stream: client,
        observation: observation.clone(),
    })
    .await
    .unwrap();
    (session, handshake.await.unwrap(), observation)
}

#[tokio::test]
async fn session_drop_interrupts_a_write_that_never_becomes_ready() {
    let (session, _peer, observation) = connected_stream().await;
    observation.block_writes.store(true, Ordering::SeqCst);
    {
        let request = session.raw.stat("/fixture");
        tokio::pin!(request);
        tokio::select! {
            result = &mut request => panic!("write unexpectedly completed: {result:?}"),
            _ = observation.write_blocked.notified() => {},
        }
    }
    drop(session);
    tokio::time::timeout(Duration::from_millis(200), observation.dropped.notified())
        .await
        .expect(
            "both protocol workers must release the underlying stream despite write backpressure",
        );
}

#[tokio::test]
async fn oversized_header_is_rejected_without_waiting_for_its_payload() {
    let (_session, mut peer, observation) = connected_stream().await;
    // Only the header is sent. A missing cap would allocate 512 KiB and then
    // keep waiting for body bytes; no multi-gigabyte allocation is attempted.
    peer.write_u32(512 * 1024).await.unwrap();
    tokio::time::timeout(Duration::from_millis(200), observation.dropped.notified())
        .await
        .expect("an oversized header must retire the stream before receiving its body");
}

#[tokio::test]
async fn session_drop_interrupts_an_already_blocked_shutdown() {
    let (session, _peer, observation) = connected_stream().await;
    observation.block_shutdown.store(true, Ordering::SeqCst);
    session.raw.close_session().unwrap();
    tokio::time::timeout(
        Duration::from_millis(200),
        observation.shutdown_blocked.notified(),
    )
    .await
    .expect("shutdown entered the underlying stream");
    drop(session);
    tokio::time::timeout(Duration::from_millis(200), observation.dropped.notified())
        .await
        .expect("cancellation releases the stream even after graceful shutdown has started");
}
