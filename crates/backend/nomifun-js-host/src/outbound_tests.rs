use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, duplex};

#[tokio::test]
async fn cancelled_partial_writes_resume_without_replay_or_reordering() {
    let mut queue = OutboundQueue::new(2, 256);
    let deadline = Instant::now() + Duration::from_secs(5);
    queue.enqueue(&"first payload", deadline, None).unwrap();
    queue.enqueue(&"second payload", deadline, None).unwrap();
    let expected = b"\"first payload\"\n\"second payload\"\n";
    let (mut writer, mut reader) = duplex(3);
    queue.write_next(&mut writer).await.unwrap();
    assert_eq!(queue.frames[0].written, 3);
    // The pipe is full. Dropping this pending write must preserve the prefix.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), queue.write_next(&mut writer))
            .await
            .is_err()
    );
    assert_eq!(queue.frames[0].written, 3);
    let output = tokio::time::timeout(Duration::from_secs(1), async {
        let (_, output) = tokio::join!(
            async {
                while !queue.is_empty() {
                    queue.write_next(&mut writer).await.unwrap();
                }
            },
            async {
                let mut output = vec![0; expected.len()];
                reader.read_exact(&mut output).await.unwrap();
                output
            }
        );
        output
    })
    .await
    .unwrap();
    assert_eq!(output, expected);
}

#[tokio::test]
async fn oversize_and_full_queue_rejections_never_write_partial_frames() {
    let mut queue = OutboundQueue::new(1, 5);
    let deadline = Instant::now() + Duration::from_secs(1);
    // JSON's escaped newline plus quotes fits four bytes, but requires five on wire.
    queue.enqueue(&"\n", deadline, None).unwrap();
    assert_eq!(
        queue.enqueue(&0, deadline, None),
        Err(JavaScriptHostError::QueueFull)
    );
    let mut bytes = Vec::new();
    while !queue.is_empty() {
        queue.write_next(&mut bytes).await.unwrap();
    }
    assert_eq!(bytes, b"\"\\n\"\n");
    assert!(matches!(
        queue.enqueue(&"\n\n", deadline, None),
        Err(JavaScriptHostError::Contract(_))
    ));
    assert!(queue.is_empty());
    // Payload alone fits, but the terminating newline would exceed the cap.
    assert!(matches!(
        queue.enqueue(&"abc", deadline, None),
        Err(JavaScriptHostError::Contract(_))
    ));
    assert!(queue.is_empty());
    queue.enqueue(&1, deadline, None).unwrap();
    while !queue.is_empty() {
        queue.write_next(&mut bytes).await.unwrap();
    }
    assert_eq!(bytes, b"\"\\n\"\n1\n");
}

#[test]
fn deadline_checks_include_frames_behind_a_later_deadline() {
    let mut queue = OutboundQueue::new(2, 32);
    let now = Instant::now();
    queue
        .enqueue(&1, now + Duration::from_secs(1), None)
        .unwrap();
    queue.enqueue(&2, now, None).unwrap();
    assert!(queue.has_expired(now));
}

#[tokio::test]
async fn closed_pipe_fails_without_retiring_the_frame() {
    let mut queue = OutboundQueue::new(1, 32);
    queue
        .enqueue(&1, Instant::now() + Duration::from_secs(1), None)
        .unwrap();
    let (mut writer, reader) = duplex(1);
    drop(reader);
    assert!(
        queue
            .write_next(&mut writer)
            .await
            .unwrap_err()
            .contains("write failed")
    );
    assert!(!queue.is_empty());
}

#[tokio::test]
async fn expired_frame_cannot_write_ahead_of_the_watchdog_tick() {
    let mut queue = OutboundQueue::new(1, 32);
    queue.enqueue(&1, Instant::now(), None).unwrap();
    let mut output = Vec::new();
    assert!(
        queue
            .write_next(&mut output)
            .await
            .unwrap_err()
            .contains("timed out")
    );
    assert!(output.is_empty());
}

#[tokio::test]
async fn service_identity_is_retired_only_after_the_whole_frame_is_written() {
    let mut queue = OutboundQueue::new(1, 32);
    let id = CorrelationId::from("service");
    queue
        .enqueue(
            &1234,
            Instant::now() + Duration::from_secs(1),
            Some(id.clone()),
        )
        .unwrap();
    let (mut writer, mut reader) = duplex(2);
    let mut output = Vec::new();
    let mut receipts = Vec::new();
    while !queue.is_empty() {
        let receipt = queue.write_next(&mut writer).await.unwrap();
        let count = if queue.is_empty() { 1 } else { 2 };
        let mut chunk = vec![0; count];
        reader.read_exact(&mut chunk).await.unwrap();
        output.extend(chunk);
        if let Some(receipt) = receipt {
            receipts.push(receipt);
            assert!(queue.is_empty());
        }
    }
    assert_eq!(output, b"1234\n");
    assert_eq!(receipts, vec![id]);
    assert_eq!(queue.write_next(&mut writer).await.unwrap(), None);
}
