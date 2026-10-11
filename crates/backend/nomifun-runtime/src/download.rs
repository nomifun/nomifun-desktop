//! Bounded download and cache publication for manifest-approved external runtimes.
use crate::managed::{
    contract::{ComponentAsset, ComponentSpec},
    provenance_bytes, report_progress,
};
use crate::progress::ToolPreparationPhase;
use crate::{ToolError, ToolId, cache, extract};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const MAX_ASSET_BYTES: u64 = 192 * 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 384 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 16_384;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const LOCK_TIMEOUT: Duration = Duration::from_secs(650);
static DOWNLOADS: Semaphore = Semaphore::const_new(2);

pub(crate) async fn supply(
    id: ToolId,
    spec: &ComponentSpec,
    asset: &ComponentAsset,
    identity: crate::embed::ExecutableIdentity,
    cancel: &CancellationToken,
) -> Result<(PathBuf, Option<String>, String), ToolError> {
    let root = cache::runtime_root()
        .ok_or_else(|| ToolError::Io(std::io::Error::other("runtime cache unavailable")))?;
    if identity.version != spec.version || identity.executable != asset.executable {
        return Err(ToolError::InvalidAsset(
            "compiled component identity disagrees with catalogue".into(),
        ));
    }
    let directory = root.join(format!(
        "{}-{}-{}",
        id.as_str(),
        spec.version,
        identity.sha256
    ));
    let client = production_client()?;
    let (path, digest) = supply_in(
        &directory,
        id,
        spec,
        asset,
        identity.sha256,
        &client,
        cancel,
    )
    .await?;
    Ok((
        dunce::canonicalize(path)?,
        Some(spec.version.clone()),
        digest,
    ))
}

fn production_client() -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("component redirect limit")
            } else if attempt.url().scheme() != "https" {
                attempt.error("component redirects must remain HTTPS")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| ToolError::Download("HTTPS client initialization failed".into()))
}

async fn acquire_lock(directory: &Path, cancel: &CancellationToken) -> Result<File, ToolError> {
    fs::create_dir_all(directory)?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("asset.lock"))?;
    let deadline = tokio::time::sleep(LOCK_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            _ = &mut deadline => return Err(ToolError::Download("another process did not finish component preparation in time".into())),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

async fn supply_in(
    directory: &Path,
    id: ToolId,
    spec: &ComponentSpec,
    asset: &ComponentAsset,
    expected_executable_digest: &str,
    client: &reqwest::Client,
    cancel: &CancellationToken,
) -> Result<(PathBuf, String), ToolError> {
    let lock = acquire_lock(directory, cancel).await?;
    // With exclusive ownership, these can only be remnants from an interrupted attempt.
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_file()
            && name.ends_with(".part")
            && (name.starts_with(".download-") || name.starts_with(".extract-"))
        {
            fs::remove_file(entry.path())?;
        }
    }
    let cached_executable = directory.join(&asset.executable);
    if cancellable_digest(&cached_executable, cancel)
        .ok()
        .as_deref()
        == Some(expected_executable_digest)
    {
        let provenance = provenance_bytes(spec, asset, expected_executable_digest);
        let mut files = vec![crate::notices::PublicationFile {
            filename: "PROVENANCE.json",
            contents: &provenance,
        }];
        files.extend(crate::notices::files(id).iter().copied());
        let path = extract::extract_file_into(
            directory,
            &asset.executable,
            &cached_executable,
            expected_executable_digest,
            &spec.version,
            id == ToolId::Bun,
            &files,
        )?;
        FileExt::unlock(&lock)?;
        return Ok((path, expected_executable_digest.into()));
    }
    let archive = directory.join("approved.asset");
    let cached = if let Ok(metadata) = fs::metadata(&archive) {
        metadata.is_file()
            && metadata.len() <= MAX_ASSET_BYTES
            && cancellable_digest(&archive, cancel)? == asset.sha256
    } else {
        false
    };
    if !cached {
        download_asset(id, client, asset, &archive, cancel).await?;
    }
    if cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    report_progress(id, ToolPreparationPhase::Extracting, 0, None);
    let directory_owned = directory.to_path_buf();
    let archive_owned = archive;
    let asset_owned = asset.clone();
    let spec_owned = spec.clone();
    let cancellation = cancel.clone();
    let expected_executable_digest = expected_executable_digest.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let mut raw = tempfile::Builder::new()
            .prefix(".extract-")
            .suffix(".part")
            .tempfile_in(&directory_owned)?;
        let digest = decode_executable(
            &archive_owned,
            &asset_owned,
            Some(raw.as_file_mut()),
            &cancellation,
        )?;
        if digest != expected_executable_digest {
            return Err(ToolError::InvalidAsset(
                "executable checksum differs from approved build identity".into(),
            ));
        }
        raw.as_file_mut().sync_all()?;
        if cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let provenance = provenance_bytes(&spec_owned, &asset_owned, &digest);
        let mut files = vec![crate::notices::PublicationFile {
            filename: "PROVENANCE.json",
            contents: &provenance,
        }];
        files.extend(crate::notices::files(id).iter().copied());
        let path = extract::extract_file_into(
            &directory_owned,
            &asset_owned.executable,
            raw.path(),
            &digest,
            &spec_owned.version,
            id == ToolId::Bun,
            &files,
        )?;
        Ok::<_, ToolError>((path, digest))
    })
    .await
    .map_err(|error| ToolError::Worker(error.to_string()))?;
    // Blocking extraction has joined and all its temporary handles have closed.
    FileExt::unlock(&lock)?;
    result
}

async fn download_asset(
    id: ToolId,
    client: &reqwest::Client,
    asset: &ComponentAsset,
    destination: &Path,
    cancel: &CancellationToken,
) -> Result<(), ToolError> {
    let _permit = tokio::select! {
        _ = cancel.cancelled() => return Err(ToolError::Cancelled),
        permit = DOWNLOADS.acquire() => permit.map_err(|_| ToolError::Download("download admission unavailable".into()))?,
    };
    report_progress(id, ToolPreparationPhase::Downloading, 0, None);
    let deadline = tokio::time::sleep(DOWNLOAD_TIMEOUT);
    tokio::pin!(deadline);
    let mut response = tokio::select! {
        _ = cancel.cancelled() => return Err(ToolError::Cancelled),
        _ = &mut deadline => return Err(ToolError::Download("approved download timed out".into())),
        response = client.get(&asset.url).send() => response.map_err(|_| ToolError::Download("approved download could not connect".into()))?,
    };
    if !response.status().is_success() {
        return Err(ToolError::Download(format!(
            "approved asset returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let total = response.content_length();
    if total.is_some_and(|length| length > MAX_ASSET_BYTES) {
        return Err(ToolError::TooLarge);
    }
    report_progress(id, ToolPreparationPhase::Downloading, 0, total);
    let temporary = tempfile::Builder::new()
        .prefix(".download-")
        .suffix(".part")
        .tempfile_in(destination.parent().unwrap())?;
    let mut file = tokio::fs::File::from_std(temporary.reopen()?);
    let mut digest = Sha256::new();
    let mut received = 0u64;
    let mut last_published = 0u64;
    let transfer: Result<(), ToolError> = async {
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            _ = &mut deadline => return Err(ToolError::Download("approved download timed out".into())),
            chunk = tokio::time::timeout(CHUNK_TIMEOUT, response.chunk()) => chunk
                .map_err(|_| ToolError::Download("approved download stopped making progress".into()))?
                .map_err(|_| ToolError::Download("approved download stream failed".into()))?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or(ToolError::TooLarge)?;
        if received > MAX_ASSET_BYTES {
            return Err(ToolError::TooLarge);
        }
        file.write_all(&chunk).await?;
        digest.update(&chunk);
        if received - last_published >= 256 * 1024 || total == Some(received) {
            report_progress(id, ToolPreparationPhase::Downloading, received, total);
            last_published = received;
        }
    }
    if total.is_some_and(|length| length != received) {
        return Err(ToolError::Download(
            "approved download was incomplete".into(),
        ));
    }
    if hex::encode(digest.finalize()) != asset.sha256 {
        return Err(ToolError::InvalidAsset("download checksum mismatch".into()));
    }
        Ok(())
    }.await;
    // Drain Tokio's pending filesystem writes before closing/deleting staging.
    let flushed = file.flush().await;
    let file = file.into_std().await;
    let synced = if transfer.is_ok() {
        file.sync_all()
    } else {
        Ok(())
    };
    drop(file);
    if let Err(error) = transfer {
        temporary.close()?;
        return Err(error);
    }
    if let Err(error) = flushed.and(synced) {
        temporary.close()?;
        return Err(error.into());
    }
    if cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    report_progress(id, ToolPreparationPhase::Downloading, received, total);
    temporary
        .persist(destination)
        .map_err(|error| ToolError::Io(error.error))?;
    Ok(())
}

fn cancellable_digest(path: &Path, cancel: &CancellationToken) -> Result<String, ToolError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        if cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn decode_executable(
    archive: &Path,
    asset: &ComponentAsset,
    mut output: Option<&mut File>,
    cancel: &CancellationToken,
) -> Result<String, ToolError> {
    let mut consume = |reader: &mut dyn Read, expected_size: u64| -> Result<String, ToolError> {
        if expected_size > MAX_EXECUTABLE_BYTES {
            return Err(ToolError::TooLarge);
        }
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            if cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            count += read as u64;
            if count > MAX_EXECUTABLE_BYTES {
                return Err(ToolError::TooLarge);
            }
            digest.update(&buffer[..read]);
            if let Some(output) = output.as_deref_mut() {
                output.write_all(&buffer[..read])?;
            }
        }
        if count != expected_size {
            return Err(ToolError::InvalidAsset(
                "executable entry was incomplete".into(),
            ));
        }
        Ok(hex::encode(digest.finalize()))
    };
    match asset.format.as_str() {
        "raw" => {
            let mut file = File::open(archive)?;
            let size = file.metadata()?.len();
            consume(&mut file, size)
        }
        "zip" => {
            let mut zip = zip::ZipArchive::new(File::open(archive)?)
                .map_err(|_| ToolError::InvalidAsset("invalid ZIP archive".into()))?;
            if zip.len() > MAX_ARCHIVE_ENTRIES {
                return Err(ToolError::TooLarge);
            }
            for index in 0..zip.len() {
                let mut entry = zip
                    .by_index(index)
                    .map_err(|_| ToolError::InvalidAsset("invalid ZIP entry".into()))?;
                if !entry.is_dir() && entry.name().rsplit('/').next() == Some(&asset.executable) {
                    if entry
                        .unix_mode()
                        .is_some_and(|mode| mode & 0o170000 == 0o120000)
                    {
                        return Err(ToolError::InvalidAsset(
                            "executable entry must not be a link".into(),
                        ));
                    }
                    let size = entry.size();
                    return consume(&mut entry, size);
                }
            }
            Err(ToolError::InvalidAsset(
                "approved ZIP has no executable".into(),
            ))
        }
        "tar.gz" => {
            let decoder = flate2::read::GzDecoder::new(File::open(archive)?);
            let mut tar = tar::Archive::new(decoder);
            for (index, entry) in tar.entries()?.enumerate() {
                if index >= MAX_ARCHIVE_ENTRIES {
                    return Err(ToolError::TooLarge);
                }
                let mut entry = entry?;
                let path = entry.path()?.into_owned();
                if entry.header().entry_type().is_file()
                    && path.ends_with(Path::new("bin").join(&asset.executable))
                {
                    let size = entry.size();
                    return consume(&mut entry, size);
                }
            }
            Err(ToolError::InvalidAsset(
                "approved TAR has no executable".into(),
            ))
        }
        _ => Err(ToolError::InvalidAsset(
            "unsupported approved archive format".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn fixture(url: String, bytes: &[u8], format: &str) -> (ComponentSpec, ComponentAsset) {
        let spec = crate::managed::specification(ToolId::Bun);
        let mut asset = spec.assets[0].clone();
        asset.url = url;
        asset.sha256 = hex::encode(Sha256::digest(bytes));
        asset.format = format.into();
        asset.executable = "bun".into();
        (spec, asset)
    }

    async fn server(bytes: Vec<u8>) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let requests = Arc::clone(&count);
        let handle = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.fetch_add(1, Ordering::SeqCst);
                let bytes = bytes.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let _ = socket.read(&mut request).await;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&bytes).await;
                });
            }
        });
        (url, count, handle)
    }

    fn fixture_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn concurrent_supply_downloads_once_and_repairs_native_cache_offline() {
        let bytes = b"approved native fixture".to_vec();
        let (url, requests, server) = server(bytes.clone()).await;
        let (spec, asset) = fixture(url, &bytes, "raw");
        let directory = tempfile::tempdir().unwrap();
        let client = fixture_client();
        let cancellation = CancellationToken::new();
        let (first, second) = tokio::join!(
            supply_in(
                directory.path(),
                ToolId::Bun,
                &spec,
                &asset,
                &asset.sha256,
                &client,
                &cancellation
            ),
            supply_in(
                directory.path(),
                ToolId::Bun,
                &spec,
                &asset,
                &asset.sha256,
                &client,
                &cancellation
            ),
        );
        let (path, digest) = first.unwrap();
        assert_eq!(second.unwrap(), (path.clone(), digest));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(&path, b"corrupt executable").unwrap();
        let repaired = supply_in(
            directory.path(),
            ToolId::Bun,
            &spec,
            &asset,
            &asset.sha256,
            &client,
            &cancellation,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(repaired.0).unwrap(), bytes);
        assert!(directory.path().join("PROVENANCE.json").is_file());
        assert!(directory.path().join("bun-1.3.13-LICENSE.md").is_file());
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
    }

    #[tokio::test]
    async fn wrong_archive_checksum_never_publishes_or_executes_the_same_named_tool() {
        let (url, _, server) = server(b"wrong same-name tool".to_vec()).await;
        let (spec, asset) = fixture(url, b"approved", "raw");
        let directory = tempfile::tempdir().unwrap();
        let result = supply_in(
            directory.path(),
            ToolId::Bun,
            &spec,
            &asset,
            &asset.sha256,
            &fixture_client(),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(ToolError::InvalidAsset(_))));
        assert!(!directory.path().join("approved.asset").exists());
        assert!(!directory.path().join("bun").exists());
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
        server.abort();
    }

    #[tokio::test]
    async fn approved_bare_executable_reuses_the_same_primary_cache_offline_without_an_archive() {
        let bytes = b"approved previous-delivery executable";
        let (spec, mut asset) = fixture(
            "http://127.0.0.1:9/unreachable".into(),
            b"archive differs from executable",
            "zip",
        );
        asset.executable = "bun".into();
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("bun"), bytes).unwrap();
        // No stamp or archive authorizes this reuse; the compiled executable pin does.
        assert!(!directory.path().join("approved.asset").exists());
        let digest = hex::encode(Sha256::digest(bytes));
        let result = supply_in(
            directory.path(),
            ToolId::Bun,
            &spec,
            &asset,
            &digest,
            &fixture_client(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result, (directory.path().join("bun"), digest));
        assert!(!directory.path().join("approved.asset").exists());
        assert!(directory.path().join("component.stamp").is_file());
        assert!(directory.path().join("PROVENANCE.json").is_file());
    }

    #[tokio::test]
    async fn downloaded_executable_must_match_the_independent_compiled_identity() {
        let bytes = b"wrong native entry with an approved archive hash";
        let (url, _, server) = server(bytes.to_vec()).await;
        let (spec, asset) = fixture(url, bytes, "raw");
        let digest = hex::encode(Sha256::digest(b"approved compiled executable"));
        let directory = tempfile::tempdir().unwrap();
        let result = supply_in(
            directory.path(),
            ToolId::Bun,
            &spec,
            &asset,
            &digest,
            &fixture_client(),
            &CancellationToken::new(),
        )
        .await;
        assert!(matches!(result, Err(ToolError::InvalidAsset(_))));
        assert!(!directory.path().join("bun").exists());
        assert!(!directory.path().join("component.stamp").exists());
        server.abort();
    }

    #[tokio::test]
    async fn cancellation_joins_the_stream_and_releases_cross_process_lock_and_staging_files() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let (started, receiving) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            socket.write_all(&vec![0u8; 32768]).await.unwrap();
            let _ = started.send(());
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let (spec, asset) = fixture(url, b"unused", "raw");
        let directory = tempfile::tempdir().unwrap();
        let cancellation = CancellationToken::new();
        let client = fixture_client();
        let producer = supply_in(
            directory.path(),
            ToolId::Bun,
            &spec,
            &asset,
            &asset.sha256,
            &client,
            &cancellation,
        );
        let cancel = async {
            receiving.await.unwrap();
            cancellation.cancel();
        };
        let (result, _) = tokio::join!(producer, cancel);
        assert!(matches!(result, Err(ToolError::Cancelled)));
        let file = File::options()
            .read(true)
            .write(true)
            .open(directory.path().join("asset.lock"))
            .unwrap();
        file.try_lock_exclusive().unwrap();
        FileExt::unlock(&file).unwrap();
        assert!(!directory.path().join("approved.asset").exists());
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
        server.abort();
    }

    #[tokio::test]
    async fn oversized_asset_is_rejected_before_any_body_is_buffered() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        MAX_ASSET_BYTES + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let (spec, asset) = fixture(url, b"unused", "raw");
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            supply_in(
                directory.path(),
                ToolId::Bun,
                &spec,
                &asset,
                &asset.sha256,
                &fixture_client(),
                &CancellationToken::new()
            )
            .await,
            Err(ToolError::TooLarge)
        ));
        assert!(!directory.path().join("approved.asset").exists());
        server.await.unwrap();
    }

    #[test]
    fn zip_and_tar_assets_extract_only_the_declared_bounded_native_entry() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"approved native";
        let zip_path = directory.path().join("bun.zip");
        let mut zip = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        zip.start_file("published/bun", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
        let (_, mut asset) = fixture("https://example.invalid".into(), b"unused", "zip");
        assert_eq!(
            decode_executable(&zip_path, &asset, None, &CancellationToken::new()).unwrap(),
            hex::encode(Sha256::digest(bytes))
        );
        asset.executable = "missing".into();
        assert!(decode_executable(&zip_path, &asset, None, &CancellationToken::new()).is_err());
        let tar_path = directory.path().join("node.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            File::create(&tar_path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(encoder);
        // Official Node archives place the binary after thousands of npm/header
        // entries (22.22.0 macOS has 6195 entries, bin/node is entry6191).
        for index in 0..5000 {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("published/lib/dependency-{index}"),
                [].as_slice(),
            )
            .unwrap();
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "published/bin/bun", bytes.as_slice())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        asset.executable = "bun".into();
        asset.format = "tar.gz".into();
        assert_eq!(
            decode_executable(&tar_path, &asset, None, &CancellationToken::new()).unwrap(),
            hex::encode(Sha256::digest(bytes))
        );
    }
}
