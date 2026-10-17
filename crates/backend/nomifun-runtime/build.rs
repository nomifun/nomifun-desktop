//! Build-time supply of the manifest-approved embedded Office renderer.
//! Downloads are authorized by components.json's upstream-published archive hashes.
#[path = "build_support.rs"]
mod build_support;
#[path = "node_distribution_contract.rs"]
mod node_distribution_contract;

use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn main() {
    for key in ["BUN_VARIANT", "NOMIFUN_EMBED_OFFICECLI"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    for file in ["build.rs", "build_support.rs", "components.json", "node_distribution_contract.rs"] {
        println!("cargo:rerun-if-changed={file}");
    }
    let target = std::env::var("TARGET").expect("cargo target");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo output directory"));
    println!("cargo:rustc-env=NOMIFUN_RUNTIME_TARGET={target}");
    let variant = std::env::var("BUN_VARIANT").unwrap_or_else(|_| "default".into());
    println!("cargo:rustc-env=NOMIFUN_RUNTIME_BUN_VARIANT={variant}");
    assert!(
        matches!(variant.as_str(), "default" | "baseline"),
        "unsupported BUN_VARIANT"
    );
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let requested = std::env::var("NOMIFUN_EMBED_OFFICECLI").ok();
    assert!(
        !release || requested.as_deref() != Some("0"),
        "release builds must embed the approved Office renderer"
    );
    let embed = release || requested.as_deref() == Some("1");
    let notices = build_support::notice_manifest();
    println!("cargo:rerun-if-changed=notices/manifest.json");
    let mut notice_metadata = String::new();
    for spec in build_support::manifest().components {
        let component_notices = notices
            .components
            .iter()
            .find(|notice| notice.id == spec.id)
            .unwrap_or_else(|| panic!("{} redistribution notices are missing", spec.id));
        build_support::validate_notices(&spec, component_notices)
            .expect("approved component notices");
        for notice in &component_notices.files {
            let path = Path::new("notices").join(&notice.filename);
            println!("cargo:rerun-if-changed={}", path.display());
            assert!(
                fs::metadata(&path).unwrap().len() <= 2 * 1024 * 1024,
                "component notice exceeds size bound"
            );
            assert_eq!(
                sha256_file(&path),
                notice.sha256,
                "{} redistribution notice checksum mismatch",
                spec.id
            );
        }
        let mut notice_values = Vec::new();
        for notice in &component_notices.files {
            let embedded_name = format!("{}-{}", spec.id, notice.filename);
            fs::copy(
                Path::new("notices").join(&notice.filename),
                out_dir.join(&embedded_name),
            )
            .unwrap();
            notice_values.push(format!("crate::notices::PublicationFile {{ filename: {:?}, contents: include_bytes!({embedded_name:?}) }}", notice.filename));
        }
        notice_metadata.push_str(&format!(
            "pub const {}_FILES: &[crate::notices::PublicationFile<'static>] = &[{}];\n",
            spec.id.to_ascii_uppercase(),
            notice_values.join(",")
        ));
        let asset = spec
            .asset(
                &target,
                if spec.id == "bun" {
                    &variant
                } else {
                    "default"
                },
            )
            .unwrap_or_else(|| {
                panic!(
                    "no approved {} asset for {target}; this release target is unsupported",
                    spec.id
                )
            });
        let meta_path = out_dir.join("officecli_meta.rs");
        if spec.delivery == build_support::ComponentDelivery::Embedded && !embed {
            build_support::write_meta_stub(&meta_path, &spec.id);
            continue;
        }
        let cache = cache_root().join(&spec.id).join(&asset.sha256);
        let archive = approved_archive(&cache, asset);
        // The trusted archive is the authority. Never trust an unverified cached executable/blob.
        let executable = out_dir.join(format!("{}.identity.bin", spec.id));
        extract_executable(&archive, &executable, &asset.format, &asset.executable);
        let digest = sha256_file(&executable);
        if spec.delivery == build_support::ComponentDelivery::Download {
            let distribution = if spec.id == "node" {
                Some(node_distribution_contract::identity(&node_distribution_contract::decode(&archive, &asset.format, None, || false).expect("valid complete Node distribution")).expect("Node distribution identity"))
            } else { None };
            fs::write(out_dir.join(format!("{}_identity.rs", spec.id)), format!(
                "// @generated identity from approved artifact\npub const SHA256: &str = {digest:?};\npub const VERSION: &str = {:?};\npub const EXECUTABLE: &str = {:?};\npub const DISTRIBUTION_SHA256: Option<&str> = {:?};\n", spec.version, asset.executable, distribution.as_deref()
            )).unwrap();
            fs::remove_file(executable).expect("remove identity scratch");
            continue;
        }
        let compressed = cache.join("component.blob.zst");
        let out_blob = out_dir.join(format!("{}.blob.zst", spec.id));
        // Cached approved archives/blobs are read-only fast paths. Scratch is
        // always Cargo's ignored/private OUT_DIR, never the shared build cache.
        if decoded_digest(&compressed).as_deref() == Some(&digest) {
            fs::copy(&compressed, &out_blob).unwrap();
        } else {
            let mut encoder =
                zstd::stream::write::Encoder::new(fs::File::create(&out_blob).unwrap(), 9).unwrap();
            std::io::copy(&mut fs::File::open(&executable).unwrap(), &mut encoder).unwrap();
            encoder.finish().unwrap().sync_all().unwrap();
        }
        fs::remove_file(executable).expect("remove embedded executable scratch");
        fs::write(meta_path, format!(
            "// @generated from components.json\npub const BLOB: &[u8] = include_bytes!(\"{}.blob.zst\");\npub const SHA256: &str = \"{digest}\";\npub const VERSION: &str = \"{}\";\npub const EXECUTABLE: &str = \"{}\";\npub const HAS_EMBEDDED: bool = true;\n",
            spec.id, spec.version, asset.executable
        )).unwrap();
    }
    fs::write(out_dir.join("component_notices.rs"), notice_metadata).unwrap();
}

fn approved_archive(cache: &Path, asset: &build_support::ComponentAsset) -> PathBuf {
    let archive = cache.join("approved.asset");
    if archive.is_file() && sha256_file(&archive) == asset.sha256 {
        return archive;
    }
    fs::create_dir_all(cache).expect("create component build cache");
    let lock = fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(cache.join("asset.lock"))
        .expect("open component build cache lock");
    lock.lock_exclusive().expect("lock component build cache");
    if !archive.is_file() || sha256_file(&archive) != asset.sha256 {
        download(&asset.url, &archive);
    }
    assert_eq!(
        sha256_file(&archive),
        asset.sha256,
        "upstream artifact checksum mismatch"
    );
    FileExt::unlock(&lock).expect("unlock component build cache");
    archive
}

fn cache_root() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
        .join("nomifun-component-cache")
}

fn download(url: &str, path: &Path) {
    let bytes = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .unwrap()
        .get(url)
        .send()
        .unwrap_or_else(|error| panic!("GET {url}: {error}"))
        .error_for_status()
        .unwrap()
        .bytes()
        .unwrap();
    let staging = path.with_extension("download");
    fs::write(&staging, bytes).unwrap();
    fs::rename(staging, path).unwrap();
}

fn extract_executable(archive: &Path, output: &Path, format: &str, name: &str) {
    let staging = output.with_extension("tmp");
    let mut file = fs::File::create(&staging).unwrap();
    match format {
        "raw" => {
            std::io::copy(&mut fs::File::open(archive).unwrap(), &mut file).unwrap();
        }
        "zip" => {
            let mut archive = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
            let mut found = false;
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).unwrap();
                if !entry.is_dir() && entry.name().rsplit('/').next() == Some(name) {
                    std::io::copy(&mut entry, &mut file).unwrap();
                    found = true;
                    break;
                }
            }
            assert!(found, "approved zip has no {name}");
        }
        "tar.gz" => {
            let decoder = flate2::read::GzDecoder::new(fs::File::open(archive).unwrap());
            let mut archive = tar::Archive::new(decoder);
            let mut found = false;
            for entry in archive.entries().unwrap() {
                let mut entry = entry.unwrap();
                let path = entry.path().unwrap().into_owned();
                if entry.header().entry_type().is_file()
                    && path.ends_with(Path::new("bin").join(name))
                {
                    std::io::copy(&mut entry, &mut file).unwrap();
                    found = true;
                    break;
                }
            }
            assert!(found, "approved tar has no bin/{name}");
        }
        _ => panic!("unsupported approved archive format"),
    }
    file.flush().unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::rename(staging, output).unwrap();
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer).unwrap();
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    hex::encode(hasher.finalize())
}

fn decoded_digest(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut decoder = zstd::stream::read::Decoder::new(file).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = decoder.read(&mut buffer).ok()?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    Some(hex::encode(hasher.finalize()))
}
