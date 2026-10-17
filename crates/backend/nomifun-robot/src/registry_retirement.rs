//! Recognize only the retired pre-authorization registry for byte-preserving
//! isolation. No old device, token, binding or thread becomes current state.

use std::fmt;
use std::path::Path;

use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

pub(super) const RETIRED_DIR: &str = "retired-pre-authorization";
const THREADS_FILE: &str = "threads.json";

// These visitors validate shape without constructing an importable retired
// record. Streaming object keys also reject duplicates that Value would erase.
struct RegistryShape;
struct RobotsShape;
struct RobotShape;

impl<'de> DeserializeSeed<'de> for RegistryShape {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for RegistryShape {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("the retired registry shape") }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        if map.next_key::<String>()?.as_deref() != Some("robots") {
            return Err(M::Error::custom("unexpected retired registry root"));
        }
        map.next_value_seed(RobotsShape)?;
        if map.next_key::<String>()?.is_some() {
            return Err(M::Error::custom("extra or duplicate retired registry root"));
        }
        Ok(())
    }
}

impl<'de> DeserializeSeed<'de> for RobotsShape {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for RobotsShape {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("retired robot shapes") }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let mut populated = false;
        while seq.next_element_seed(RobotShape)?.is_some() { populated = true; }
        if !populated { return Err(S::Error::custom("an empty registry needs no retirement")); }
        Ok(())
    }
}

impl<'de> DeserializeSeed<'de> for RobotShape {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for RobotShape {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("the exact ten-field retired robot shape") }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        const FIELDS: &[&str] = &["robot_id", "client_id", "name", "companion_id", "token_hash",
            "activation_code", "board", "firmware_version", "last_seen", "created_at"];
        let mut seen = 0u16;
        while let Some(key) = map.next_key::<String>()? {
            let Some(index) = FIELDS.iter().position(|field| *field == key) else {
                return Err(M::Error::custom("unknown retired robot field"));
            };
            let bit = 1 << index;
            if seen & bit != 0 { return Err(M::Error::custom("duplicate retired robot field")); }
            seen |= bit;
            let value = map.next_value::<Value>()?;
            let valid = match key.as_str() {
                "companion_id" | "activation_code" => value.is_null() || value.is_string(),
                "last_seen" => value.is_null() || value.is_i64(),
                "created_at" => value.is_i64(),
                _ => value.is_string(),
            };
            if !valid { return Err(M::Error::custom("invalid retired robot field type")); }
        }
        if seen != (1 << FIELDS.len()) - 1 { return Err(M::Error::custom("incomplete retired robot shape")); }
        Ok(())
    }
}

fn is_pre_authorization_registry(bytes: &[u8]) -> bool {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    RegistryShape.deserialize(&mut deserializer).is_ok() && deserializer.end().is_ok()
}

fn is_indirected(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return true; } // FILE_ATTRIBUTE_REPARSE_POINT
    }
    false
}

async fn regular_file_exists(path: &Path) -> anyhow::Result<bool> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() && !is_indirected(&metadata) => Ok(true),
        Ok(_) => anyhow::bail!("robot retirement source must be a regular file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Only an exact retired shape authorizes this domain-local clean start.
/// Threads move first and the registry moves last, so an interrupted operation
/// can retry from the still-active old registry without a second startup ledger.
pub(super) async fn retire_pre_authorization_registry(dir: &Path, bytes: &[u8]) -> anyhow::Result<bool> {
    if !is_pre_authorization_registry(bytes) { return Ok(false); }
    let directory_metadata = tokio::fs::symlink_metadata(dir).await?;
    if !directory_metadata.is_dir() || is_indirected(&directory_metadata) {
        anyhow::bail!("robot retirement source must be a real directory");
    }
    let registry = dir.join(super::ROBOTS_FILE);
    let threads = dir.join(THREADS_FILE);
    if !regular_file_exists(&registry).await? { anyhow::bail!("retired robot registry disappeared before isolation"); }
    let has_threads = regular_file_exists(&threads).await?;
    let retired = dir.join(RETIRED_DIR);
    match tokio::fs::create_dir(&retired).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = tokio::fs::symlink_metadata(&retired).await?;
            if !metadata.is_dir() || is_indirected(&metadata) {
                anyhow::bail!("robot retirement destination must be a directory");
            }
        }
        Err(error) => return Err(error.into()),
    }
    // Preflight every move before changing a source. A prior thread-only move
    // is resumable; an existing destination for an active source is a conflict.
    for (source, present) in [(&registry, true), (&threads, has_threads)] {
        if present {
            match tokio::fs::symlink_metadata(retired.join(source.file_name().unwrap())).await {
                Ok(_) => anyhow::bail!("robot retirement destination already exists; refusing to overwrite {}", source.file_name().unwrap().to_string_lossy()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    if has_threads {
        nomifun_common::publish_new_file_noreplace(&threads, &retired.join(THREADS_FILE))?;
    }
    nomifun_common::publish_new_file_noreplace(&registry, &retired.join(super::ROBOTS_FILE))?;
    tracing::info!("robot: isolated the retired pre-authorization registry and threads; pairing starts empty");
    Ok(true)
}
