//! NTFS stream publication keeps the existing base file and every other
//! stream intact. Windows only atomically replaces an absent or empty stream;
//! a populated stream retains the native ERROR_INVALID_PARAMETER result.
use std::{fs::OpenOptions, io::Write, mem, path::{Path, PathBuf}};
use std::os::windows::{ffi::OsStrExt as _, fs::{MetadataExt as _, OpenOptionsExt as _}, io::AsRawHandle as _};
use nomifun_common::AppError;
use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_WRITE, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_RENAME_INFO, FileRenameInfo, SetFileInformationByHandle};
use crate::{agent_patch_outcome::PatchPublicationFailure, service::PublicationSource};

fn parts(path: &Path) -> Option<(PathBuf, String)> {
    let name = path.file_name()?.to_str()?;
    let (base, stream) = name.split_once(':')?;
    if base.is_empty() || stream.is_empty() || stream.starts_with(':') { return None; }
    let base = nomi_native_files::windows::join_component(path.parent()?, std::ffi::OsStr::new(base)).ok()?;
    Some((base, format!(":{stream}")))
}
pub(crate) fn is_named_stream(path: &Path) -> bool { parts(path).is_some() }

/// A non-colon FileRenameInfo target would rename the base file even when
/// its source handle is a named stream. Only a same-file stream rename uses
/// the documented leading-colon form; never let it mutate the base name.
pub(crate) fn rename_entry(file: &std::fs::File, source: &Path, target: &Path) -> std::io::Result<()> {
    let (Some((source_base, _)), Some((target_base, stream))) = (parts(source), parts(target)) else {
        return Err(std::io::Error::from_raw_os_error(87));
    };
    if source_base != target_base { return Err(std::io::Error::from_raw_os_error(87)); }
    rename(file, &stream, false)
}

pub(crate) fn publish(
    path: &Path, data: &[u8], source: PublicationSource<'_>, after_staging: impl FnOnce(),
    after_publication: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), PatchPublicationFailure> {
    let (base, stream) = parts(path).ok_or_else(|| AppError::BadRequest("invalid named stream path".into()))?;
    let io = |error| crate::path_safety::native_io_error("publishing native file stream", error);
    // Deny rename/deletion of the base while creating and renaming its stream.
    // Metadata access does not require listing or reading its default bytes.
    let base_file = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&base).map_err(io)?;
    let metadata = base_file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || nomi_native_files::windows::final_path(&base_file).map_err(io)? != base {
        return Err(AppError::Conflict("stream base changed identity or is not a regular file".into()).into());
    }
    let existing = match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io(error).into()),
    };
    if matches!(source, PublicationSource::Absent) != existing.is_none() {
        return Err(AppError::Conflict("file stream existence changed before publication".into()).into());
    }
    if let Some(expected) = source.expected_bytes() {
        let observed = nomi_native_files::acquire_source_path(path, nomi_native_files::MAX_SOURCE_BYTES)
            .map_err(crate::native_file_acquisition_error)?;
        if observed.is_none_or(|value| value.bytes != expected) {
            return Err(AppError::Conflict("file stream changed before publication".into()).into());
        }
    }
    // FileRenameInformation documents this restriction, and the native rename
    // rejects it with 87. Do not truncate, delete or copy over existing bytes.
    if existing.as_ref().is_some_and(|metadata| metadata.len() != 0) {
        return Err(io(std::io::Error::from_raw_os_error(87)).into());
    }
    let staging = nomi_native_files::windows::join_component(base.parent().unwrap(), std::ffi::OsStr::new(
        &format!("{}:.nomifun-stream-{}", base.file_name().unwrap().to_string_lossy(), nomifun_common::generate_id())))
        .map_err(io)?;
    let mut file = OpenOptions::new().write(true).create_new(true)
        .access_mode(FILE_GENERIC_WRITE | DELETE).share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&staging).map_err(io)?;
    let mut published = false;
    let result = (|| {
        file.write_all(data).and_then(|()| file.sync_all()).map_err(io)?;
        after_staging();
        rename(&file, &stream, existing.is_some()).map_err(io)?;
        published = true;
        file.sync_all().map_err(io)?;
        if file.metadata().map_err(io)?.len() != data.len() as u64
            || nomi_native_files::windows::final_path(&file).map_err(io)? != path {
            return Err(AppError::Conflict("published file stream could not be verified".into()));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let temporary_cleanup_unconfirmed = if published { drop(file); false }
            else { crate::windows_cleanup::remove_handle(file, true).is_err() };
        return Err(PatchPublicationFailure { error, published, publication_verified: false,
            publication_identity: None, temporary_cleanup_unconfirmed });
    }
    drop(file);
    after_publication().map_err(|error| PatchPublicationFailure { error, published: true, publication_verified: true,
        publication_identity: None, temporary_cleanup_unconfirmed: false })
}

fn rename(file: &std::fs::File, target: &str, replace: bool) -> std::io::Result<()> {
    let name: Vec<u16> = std::ffi::OsStr::new(target).encode_wide().collect();
    let offset = mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + (name.len() + 1) * 2;
    let mut buffer = vec![0_usize; size.div_ceil(mem::size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the aligned buffer contains the complete checked stream name;
    // Windows interprets the leading colon relative to this same file object.
    let ok = unsafe {
        (*info).Anonymous.ReplaceIfExists = replace;
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), buffer.as_mut_ptr().cast::<u8>().add(offset).cast(), name.len());
        SetFileInformationByHandle(file.as_raw_handle(), FileRenameInfo, info.cast(), size as u32)
    };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}
