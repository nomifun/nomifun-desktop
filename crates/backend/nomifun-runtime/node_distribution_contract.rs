//! Canonical identity and safe extraction of the complete official Node distribution.
//! Shared by the build pin generator and the runtime supplier.
#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const MAX_ENTRIES: usize = 32_768;
const MAX_BYTES: u64 = 768 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct Entry {
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub size: u64,
    pub digest: String,
    pub target: Option<String>,
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn identity(entries: &[Entry]) -> io::Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(entries)?)))
}

fn portable_path(value: &str) -> io::Result<PathBuf> {
    if value.is_empty() || value.contains(['\\', ':', '\0']) {
        return Err(invalid("invalid distribution path"));
    }
    let path = Path::new(value);
    if path.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(invalid("distribution path is not a relative normal path"));
    }
    Ok(path.to_path_buf())
}

fn stripped_path(value: &str, root: &mut Option<String>) -> io::Result<Option<String>> {
    let value = value.trim_end_matches('/');
    let path = portable_path(value)?;
    let mut parts = path.iter();
    let prefix = parts.next().unwrap().to_str().ok_or_else(|| invalid("non-UTF8 archive path"))?;
    match root {
        Some(root) if root != prefix => return Err(invalid("distribution has multiple roots")),
        None => *root = Some(prefix.to_owned()),
        _ => {}
    }
    let relative: PathBuf = parts.collect();
    Ok((!relative.as_os_str().is_empty()).then(|| relative.to_string_lossy().replace('\\', "/")))
}

fn link_destination(path: &str, target: &str, hard: bool) -> io::Result<String> {
    if target.is_empty() || target.contains(['\\', ':', '\0']) || Path::new(target).is_absolute() {
        return Err(invalid("distribution link has an invalid target"));
    }
    let mut parts: Vec<String> = if hard {
        Vec::new()
    } else {
        Path::new(path).parent().unwrap().iter().map(|part| part.to_string_lossy().into_owned()).collect()
    };
    for part in Path::new(target).components() {
        match part {
            Component::CurDir => {}
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::ParentDir if !parts.is_empty() => { parts.pop(); }
            _ => return Err(invalid("distribution link escapes its root")),
        }
    }
    if parts.is_empty() { return Err(invalid("distribution link targets its root")); }
    Ok(parts.join("/"))
}

struct Decoder<'a, F> {
    output: Option<&'a Path>,
    cancelled: F,
    entries: BTreeMap<String, Entry>,
    root: Option<String>,
    bytes: u64,
}

impl<F: Fn() -> bool> Decoder<'_, F> {
    fn check(&self) -> io::Result<()> {
        if (self.cancelled)() { Err(io::Error::new(io::ErrorKind::Interrupted, "distribution preparation cancelled")) } else { Ok(()) }
    }

    fn entry(&mut self, name: &str, kind: &str, mode: u32, size: u64, target: Option<String>, mut data: impl Read) -> io::Result<()> {
        self.check()?;
        let Some(path) = stripped_path(name, &mut self.root)? else {
            return if kind == "directory" { Ok(()) } else { Err(invalid("distribution root is not a directory")) };
        };
        if self.entries.len() >= MAX_ENTRIES || self.entries.contains_key(&path) {
            return Err(invalid("distribution entry limit or duplicate path"));
        }
        // No archive entry may write through a link, regardless of archive ordering.
        for ancestor in Path::new(&path).ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()) {
            if self.entries.get(&ancestor.to_string_lossy().into_owned()).is_some_and(|entry| entry.kind != "directory") {
                return Err(invalid("distribution entry has a non-directory ancestor"));
            }
        }
        if kind != "directory" && self.entries.keys().any(|existing| existing.starts_with(&format!("{path}/"))) {
            return Err(invalid("distribution entry replaces an ancestor"));
        }
        let mode = mode & 0o777;
        let destination = self.output.map(|root| root.join(&path));
        let mut digest = Sha256::new();
        if kind == "file" {
            self.bytes = self.bytes.checked_add(size).ok_or_else(|| invalid("distribution size overflow"))?;
            if self.bytes > MAX_BYTES { return Err(invalid("distribution exceeds size limit")); }
            let mut output = if let Some(destination) = &destination {
                fs::create_dir_all(destination.parent().unwrap())?;
                Some(File::options().write(true).create_new(true).open(destination)?)
            } else { None };
            let mut count = 0u64;
            let mut buffer = [0; 65536];
            loop {
                self.check()?;
                let read = data.read(&mut buffer)?;
                if read == 0 { break; }
                count += read as u64;
                if count > size { return Err(invalid("distribution entry exceeds declared size")); }
                digest.update(&buffer[..read]);
                if let Some(output) = &mut output { output.write_all(&buffer[..read])?; }
            }
            if count != size { return Err(invalid("incomplete distribution entry")); }
            if let Some(output) = output { output.sync_all()?; }
            if let Some(destination) = &destination { set_mode(destination, mode)?; }
        } else if kind == "directory" {
            if let Some(destination) = destination { fs::create_dir_all(destination)?; }
        } else if kind == "symlink" || kind == "hardlink" {
            link_destination(&path, target.as_deref().ok_or_else(|| invalid("link lacks target"))?, kind == "hardlink")?;
        } else { return Err(invalid("unsupported special distribution entry")); }
        self.entries.insert(path.clone(), Entry {
            path, kind: kind.into(), mode: if kind == "symlink" { 0o777 } else { mode },
            size: if kind == "file" { size } else { 0 },
            digest: if kind == "file" { hex::encode(digest.finalize()) } else { String::new() }, target,
        });
        Ok(())
    }

    fn finish(mut self) -> io::Result<Vec<Entry>> {
        self.check()?;
        // Links are created only after every ordinary file and directory, so no file
        // extraction can traverse a previously created symlink.
        let mut hardlink_contents = Vec::new();
        for entry in self.entries.values().filter(|entry| entry.kind.ends_with("link")) {
            self.check()?;
            let target = entry.target.as_deref().unwrap();
            let destination = link_destination(&entry.path, target, entry.kind == "hardlink")?;
            let mut resolved = destination.clone();
            let mut seen = std::collections::BTreeSet::new();
            loop {
                if !seen.insert(resolved.clone()) { return Err(invalid("cyclic distribution link")); }
                let candidate = self.entries.get(&resolved).ok_or_else(|| invalid("distribution link target is absent"))?;
                if !candidate.kind.ends_with("link") { break; }
                resolved = link_destination(&candidate.path, candidate.target.as_deref().unwrap(), candidate.kind == "hardlink")?;
            }
            if entry.kind == "hardlink" {
                let source = &self.entries[&resolved];
                if source.kind != "file" { return Err(invalid("hardlink target is not a regular file")); }
                hardlink_contents.push((entry.path.clone(), source.size, source.digest.clone(), source.mode));
            }
            if let Some(root) = self.output {
                let path = root.join(&entry.path);
                fs::create_dir_all(path.parent().unwrap())?;
                if entry.kind == "hardlink" {
                    if self.entries[&resolved].kind != "file" { return Err(invalid("hardlink target is not a regular file")); }
                    fs::hard_link(root.join(&resolved), path)?;
                } else { create_symlink(target, &path)?; }
            }
        }
        for (path, size, digest, mode) in hardlink_contents {
            let entry = self.entries.get_mut(&path).unwrap();
            entry.size = size; entry.digest = digest; entry.mode = mode;
        }
        if let Some(root) = self.output {
            for entry in self.entries.values().filter(|entry| entry.kind == "directory").rev() {
                set_mode(&root.join(&entry.path), entry.mode)?;
            }
        }
        Ok(self.entries.into_values().collect())
    }
}

pub fn decode(archive: &Path, format: &str, output: Option<&Path>, cancelled: impl Fn() -> bool) -> io::Result<Vec<Entry>> {
    let mut decoder = Decoder { output, cancelled, entries: BTreeMap::new(), root: None, bytes: 0 };
    match format {
        "tar.gz" => {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
            for (index, entry) in archive.entries()?.enumerate() {
                if index >= MAX_ENTRIES { return Err(invalid("distribution entry limit")); }
                let mut entry = entry?;
                let path = entry.path()?.to_str().ok_or_else(|| invalid("non-UTF8 archive path"))?.to_owned();
                let entry_type = entry.header().entry_type();
                let kind = if entry_type.is_file() { "file" } else if entry_type.is_dir() { "directory" }
                    else if entry_type.is_symlink() { "symlink" } else if entry_type.is_hard_link() { "hardlink" }
                    else { return Err(invalid("unsupported special distribution entry")); };
                let mut target = entry.link_name()?.map(|path| path.to_string_lossy().into_owned());
                if kind == "hardlink" { target = Some(stripped_path(target.as_deref().unwrap_or(""), &mut decoder.root)?.ok_or_else(|| invalid("hardlink targets root"))?); }
                decoder.entry(&path, kind, entry.header().mode()?, entry.size(), target, &mut entry)?;
            }
        }
        "zip" => {
            let mut archive = zip::ZipArchive::new(File::open(archive)?)?;
            if archive.len() > MAX_ENTRIES { return Err(invalid("distribution entry limit")); }
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index)?;
                let name = entry.name().to_owned();
                let mode = entry.unix_mode().unwrap_or(if entry.is_dir() { 0o755 } else { 0o644 });
                if !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000 | 0o120000) {
                    return Err(invalid("unsupported special distribution entry"));
                }
                let kind = if entry.is_dir() { "directory" } else if mode & 0o170000 == 0o120000 { "symlink" } else { "file" };
                let target = if kind == "symlink" {
                    if entry.size() > 4096 { return Err(invalid("distribution link target too long")); }
                    let mut target = String::new(); entry.read_to_string(&mut target)?; Some(target)
                } else { None };
                decoder.entry(&name, kind, mode, entry.size(), target, &mut entry)?;
            }
        }
        _ => return Err(invalid("Node requires a complete official archive")),
    }
    decoder.finish()
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(path, fs::Permissions::from_mode(mode))?; }
    #[cfg(not(unix))] let _ = (path, mode);
    Ok(())
}
fn create_symlink(target: &str, path: &Path) -> io::Result<()> {
    #[cfg(unix)] { std::os::unix::fs::symlink(target, path) }
    #[cfg(windows)] { std::os::windows::fs::symlink_file(target, path) }
    #[cfg(not(any(unix, windows)))] { let _ = (target, path); Err(invalid("symlinks unavailable on this platform")) }
}

/// Compare all immutable payload bytes, links and executable bits to the build-pinned index.
pub fn validate(root: &Path, entries: &[Entry], metadata_files: &[&str], cancelled: impl Fn() -> bool) -> io::Result<bool> {
    if entries.len() > MAX_ENTRIES { return Ok(false); }
    if !fs::symlink_metadata(root)?.is_dir() { return Ok(false); }
    let mut expected = std::collections::BTreeSet::new();
    for entry in entries {
        let path = portable_path(&entry.path)?;
        for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
            expected.insert(ancestor.to_path_buf());
        }
    }
    expected.extend(metadata_files.iter().map(PathBuf::from));
    let mut directories = vec![PathBuf::new()];
    let mut count = 0;
    while let Some(relative) = directories.pop() {
        if cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "distribution verification cancelled")); }
        for child in fs::read_dir(root.join(&relative))? {
            let child = child?;
            let path = relative.join(child.file_name());
            count += 1;
            if count > MAX_ENTRIES + metadata_files.len() || !expected.contains(&path) { return Ok(false); }
            if child.file_type()?.is_dir() { directories.push(path); }
        }
    }
    for entry in entries {
        if cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "distribution verification cancelled")); }
        let relative = portable_path(&entry.path)?;
        let path = root.join(relative);
        let metadata = match fs::symlink_metadata(&path) { Ok(metadata) => metadata, Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false), Err(error) => return Err(error) };
        match entry.kind.as_str() {
            "file" | "hardlink" if metadata.is_file() => {
                {
                    if metadata.len() != entry.size { return Ok(false); }
                    let mut file = File::open(&path)?;
                    let mut hash = Sha256::new();
                    let mut bytes = [0; 65536];
                    loop { if cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "distribution verification cancelled")); } let count = file.read(&mut bytes)?; if count == 0 { break; } hash.update(&bytes[..count]); }
                    if hex::encode(hash.finalize()) != entry.digest { return Ok(false); }
                }
                #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; if metadata.permissions().mode() & 0o777 != entry.mode { return Ok(false); } }
            }
            "directory" if metadata.is_dir() => {
                #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; if metadata.permissions().mode() & 0o777 != entry.mode { return Ok(false); } }
            }
            "symlink" if metadata.file_type().is_symlink() => {
                if fs::read_link(path)?.to_str() != entry.target.as_deref() { return Ok(false); }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}
