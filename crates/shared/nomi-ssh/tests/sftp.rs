//! SFTP file operations against a real sshd (internal-sftp subsystem).
mod support;

#[tokio::test(flavor = "multi_thread")]
async fn write_then_read_roundtrips() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let fs = support::connect(&sshd).await.open_sftp().await.expect("sftp");
    let path = format!("/tmp/nomi_sftp_{}.txt", std::process::id());
    fs.write_file_atomic(&path, b"line1\nline2\n")
        .await
        .expect("write");
    let back = fs.read_file(&path).await.expect("read");
    assert_eq!(back, b"line1\nline2\n");
    let _ = fs.remove_file(&path).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn atomic_overwrite_replaces_content() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let fs = support::connect(&sshd).await.open_sftp().await.unwrap();
    let path = format!("/tmp/nomi_sftp_ovw_{}.txt", std::process::id());
    fs.write_file_atomic(&path, b"first").await.expect("write 1");
    // Overwrite an existing file — exercises the rename-onto-existing path.
    fs.write_file_atomic(&path, b"second_longer").await.expect("write 2");
    let back = fs.read_file(&path).await.expect("read");
    assert_eq!(back, b"second_longer");
    let st = fs.stat(&path).await.expect("stat");
    assert_eq!(st.size, "second_longer".len() as u64);
    assert!(!st.is_dir);
    let _ = fs.remove_file(&path).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stat_and_list_work() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let fs = support::connect(&sshd).await.open_sftp().await.unwrap();
    let st = fs.stat("/tmp").await.expect("stat /tmp");
    assert!(st.is_dir, "/tmp should be a directory");
    let entries = fs.list_dir("/tmp").await.expect("list /tmp");
    // /tmp always has entries in a running system; at minimum it should not error.
    let _ = entries;
    let canon = fs.canonicalize("/tmp/../tmp").await.expect("canonicalize");
    assert_eq!(canon, "/tmp", "canonicalize should resolve to /tmp, got {canon}");
}
