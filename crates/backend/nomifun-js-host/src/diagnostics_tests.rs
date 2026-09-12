use super::*;
use tokio::io::{AsyncWriteExt, duplex};

#[tokio::test]
async fn stderr_diagnostics_keep_counts_without_retaining_raw_content() {
    let (mut writer, reader) = duplex(64);
    let mut tasks = JoinSet::new();
    tasks.spawn(drain_stderr(reader));
    writer
        .write_all(b"fixture-secret\nsecond line\n")
        .await
        .unwrap();
    drop(writer);
    let result = finish_stderr(&mut tasks, Instant::now() + Duration::from_secs(1)).await;
    assert_eq!(result, "; stderr contained 27 bytes across 2 lines");
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn stderr_with_an_open_writer_cannot_block_terminal_state_forever() {
    let (_writer, reader) = duplex(64);
    let mut tasks = JoinSet::new();
    tasks.spawn(drain_stderr(reader));
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        finish_stderr(&mut tasks, Instant::now() + Duration::from_millis(20)),
    )
    .await
    .unwrap();
    assert_eq!(result, "; stderr drain timed out");
    assert!(tasks.is_empty(), "collector must be aborted and joined");
}

#[tokio::test]
async fn dropping_startup_diagnostics_cancels_the_collector() {
    let (mut writer, reader) = duplex(64);
    let mut tasks = JoinSet::new();
    tasks.spawn(drain_stderr(reader));
    drop(tasks);
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if writer.write_all(b"x").await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "cancelled startup retained its stderr reader"
    );
}
