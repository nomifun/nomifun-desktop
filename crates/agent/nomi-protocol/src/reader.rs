use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::sync::mpsc;

use crate::commands::ProtocolCommand;

// Allow large history imports while bounding queued and unfinished input.
const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 8;

/// Reads JSON Lines with bounded buffering. Oversized input closes the stream.
/// Dropping the receiver stops the async reader; Tokio's OS stdin read itself
/// may remain blocked until input or process exit.
pub fn spawn_stdin_reader() -> mpsc::Receiver<ProtocolCommand> {
    let (tx, rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    tokio::spawn(read_commands(BufReader::new(tokio::io::stdin()), tx));
    rx
}

async fn read_commands(mut reader: impl AsyncBufRead + Unpin, tx: mpsc::Sender<ProtocolCommand>) {
    let mut line = String::new();

    loop {
        line.clear();
        let mut limited = (&mut reader).take(MAX_COMMAND_BYTES as u64 + 1);
        let result = tokio::select! {
            biased;
            _ = tx.closed() => break,
            result = limited.read_line(&mut line) => result,
        };
        match result {
            Ok(0) => break, // EOF - client closed stdin
            Ok(n) if n > MAX_COMMAND_BYTES => {
                tracing::warn!(target: "nomi_protocol", "protocol command exceeds size limit");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<ProtocolCommand>(trimmed) {
                    Ok(cmd) => {
                        if tx.send(cmd).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        // Serde diagnostics can include user-supplied field values.
                        tracing::debug!(target: "nomi_protocol", line = e.line(), column = e.column(),
                            category = ?e.classify(), "invalid protocol command");
                    }
                }
            }
            Err(e) => {
                tracing::debug!(target: "nomi_protocol", error = %e, "stdin read error");
                break;
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn commands_preserve_order_and_final_line_without_newline() {
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        read_commands(&b"\ninvalid\n{\"type\":\"ping\"}\r\n{\"type\":\"stop\"}"[..], tx).await;
        assert_eq!(rx.recv().await, Some(ProtocolCommand::Ping));
        assert_eq!(rx.recv().await, Some(ProtocolCommand::Stop));
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn receiver_drop_stops_pending_read() {
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let (_peer, stream) = tokio::io::duplex(64);
        drop(rx);
        tokio::time::timeout(Duration::from_millis(100), read_commands(BufReader::new(stream), tx))
            .await.expect("closed receiver must stop reader");
    }

    #[tokio::test]
    async fn command_queue_applies_backpressure() {
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let input = "{\"type\":\"ping\"}\n".repeat(32);
        let read = read_commands(input.as_bytes(), tx);
        tokio::pin!(read);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut read).await.is_err());
        assert_eq!(rx.recv().await, Some(ProtocolCommand::Ping));
        drop(rx);
        tokio::time::timeout(Duration::from_millis(100), &mut read).await.unwrap();
    }

    #[tokio::test]
    async fn oversized_line_terminates_without_waiting_for_newline() {
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let mut oversized = vec![b'x'; 16 * 1024 * 1024 + 1];
        oversized.extend_from_slice(b"\n{\"type\":\"ping\"}\n");
        read_commands(oversized.as_slice(), tx).await;
        assert_eq!(rx.recv().await, None, "oversized protocol input must terminate the reader");
    }

    #[tokio::test]
    async fn limit_applies_even_without_newline_or_eof() {
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let input = BufReader::new(tokio::io::repeat(b'x'));
        tokio::time::timeout(Duration::from_secs(2), read_commands(input, tx)).await.unwrap();
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test]
    async fn exact_size_limit_accepts_a_complete_command() {
        let (tx, mut rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let mut input = br#"{"type":"ping"}"#.to_vec();
        input.resize(MAX_COMMAND_BYTES - 1, b' ');
        input.push(b'\n');
        read_commands(input.as_slice(), tx).await;
        assert_eq!(rx.recv().await, Some(ProtocolCommand::Ping));
        assert_eq!(rx.recv().await, None);
    }
}
