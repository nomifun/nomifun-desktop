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

/// An atomic write starts by creating a sibling temp file, so a missing *parent
/// directory* fails on a path the caller never named. Reported bare, the SFTP
/// status reads "no such file" about `/srv/app/config.yml` — and the natural
/// misreading is "create the file", when the fix is `mkdir -p /srv/app`. The
/// message has to carry the directory and say which file could not be created.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_into_a_missing_directory_names_the_directory() {
    let Some(sshd) = support::start_pubkey_sshd() else {
        eprintln!("SKIP: no usable sshd");
        return;
    };
    let fs = support::connect(&sshd).await.open_sftp().await.unwrap();
    let dir = format!("/tmp/nomi_absent_dir_{}", std::process::id());
    let err = fs
        .write_file_atomic(&format!("{dir}/config.yml"), b"key: value\n")
        .await
        .expect_err("writing under a missing directory must fail");

    let msg = err.to_string();
    assert!(
        msg.contains(&dir),
        "the message must name the directory that is missing, got: {msg}"
    );
    assert!(
        msg.contains("mkdir"),
        "the message must point at the actual fix, got: {msg}"
    );
    assert!(
        msg.contains("temporary"),
        "the message must say the failure was creating the temp file, not the \
         target, got: {msg}"
    );
}
