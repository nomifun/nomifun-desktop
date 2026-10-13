//! Atomic workspace publication relative to the prepared parent's capability.
//! POSIX name replacement is not an atomic compare-and-swap; this owner keeps
//! that existing limit explicit while preventing ambient ancestor redirects.

use std::{ffi::{CString, OsStr}, fs::File, io::{Read, Seek, SeekFrom}, os::{fd::AsRawFd, unix::{ffi::OsStrExt, fs::MetadataExt as _}}};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, Metadata, MetadataExt as _, OpenOptions, OpenOptionsExt as _};
use nomifun_common::AppError;

use crate::{agent_patch_outcome::PatchPublicationFailure, publication_identity::{self, PublicationIdentity}, service::PublicationSource, workspace_write::PreparedParent};

fn conflict(message: &str) -> AppError { AppError::Conflict(message.into()) }
fn io_error(error: std::io::Error) -> AppError { AppError::Internal(format!("workspace publication failed: {error}")) }

fn matches_named(identity: &PublicationIdentity, directory: &Dir, name: &OsStr) -> std::io::Result<bool> {
    let opened = identity.metadata()?;
    let named = directory.symlink_metadata(name)?;
    Ok(named.is_file() && opened.dev() == named.dev() && opened.ino() == named.ino())
}

fn matches_metadata(file: &File, expected: &Metadata) -> std::io::Result<bool> {
    let actual = file.metadata()?;
    Ok(actual.is_file() && actual.dev() == expected.dev() && actual.ino() == expected.ino())
}

fn matches_bytes(file: &mut File, expected: &[u8]) -> std::io::Result<bool> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(expected.len().saturating_add(1) as u64).read_to_end(&mut bytes)?;
    Ok(bytes == expected)
}

fn rename_noreplace(directory: &Dir, source: &OsStr, target: &OsStr) -> std::io::Result<()> {
    let name = |value: &OsStr| CString::new(value.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "publication name contains NUL"));
    let source = name(source)?;
    let target = name(target)?;
    // SAFETY: the borrowed directory and both NUL-terminated single-component
    // names remain live. The syscall never resolves an ambient ancestor.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renameatx_np(directory.as_raw_fd(), source.as_ptr(),
        directory.as_raw_fd(), target.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let result = unsafe { libc::syscall(libc::SYS_renameat2, directory.as_raw_fd(), source.as_ptr(),
        directory.as_raw_fd(), target.as_ptr(), libc::RENAME_NOREPLACE) };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "atomic relative publication is unavailable"));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

pub(crate) fn publish(
    parent: &PreparedParent,
    data: &[u8],
    source: PublicationSource<'_>,
) -> Result<Option<PublicationIdentity>, PatchPublicationFailure> {
    publish_with_hooks(parent, data, source, || {}, || {}, || Ok(()))
}

fn publish_with_hooks(
    parent: &PreparedParent, data: &[u8], source: PublicationSource<'_>,
    after_staging: impl FnOnce(), before_replace: impl FnOnce(),
    after_publication: impl FnOnce() -> Result<(), AppError>,
) -> Result<Option<PublicationIdentity>, PatchPublicationFailure> {
    use std::io::Write;
    parent.verify()?;
    let name = parent.as_path().file_name().ok_or_else(|| conflict("publication target has no filename"))?;
    let directory = parent.directory();
    // Darwin fsync accepts the search-only descriptor (including mode 0300
    // parents), verified by the native regression below. Linux O_PATH cannot
    // be synced: acquire the readable directory fd required by the existing
    // durability contract before any staging/publication side effect.
    let sync_directory = crate::unix_directory::open_for_sync(directory).map_err(io_error)?;
    let temporary = format!(".nomifun-patch-{}-{}.tmp", std::process::id(), nomifun_common::generate_id());
    let temporary = OsStr::new(&temporary);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true).mode(0o600).follow(FollowSymlinks::No);
    let mut staged = directory.open_with(temporary, &options).map_err(io_error)?.into_std();
    let identity = publication_identity::capture(&staged).map_err(|error| PatchPublicationFailure {
        error: io_error(error), published: false, publication_verified: false,
        publication_identity: None, temporary_cleanup_unconfirmed: true,
    })?;
    let publication_identity = (!matches!(source, PublicationSource::Absent)).then(|| identity.clone());
    let mut published = false;
    let mut verified = false;
    let mut consumed = false;
    let result = (|| -> Result<(), AppError> {
        staged.write_all(data).map_err(io_error)?;
        staged.sync_all().map_err(io_error)?;
        after_staging();
        parent.verify()?;
        let metadata = match directory.symlink_metadata(name) {
            Ok(metadata) if metadata.is_file() => Some(metadata),
            Ok(_) => return Err(conflict("publication target changed to a non-regular file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(error)),
        };
        let writable = if !matches!(source, PublicationSource::Absent) {
            let metadata = metadata.as_ref().ok_or_else(|| conflict("publication target disappeared"))?;
            let mut options = OpenOptions::new();
            options.write(true).follow(FollowSymlinks::No);
            let writable = directory.open_with(name, &options).map_err(|error| {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    AppError::Forbidden(format!("workspace publication target is not writable: {error}"))
                } else { io_error(error) }
            })?.into_std();
            if !matches_metadata(&writable, metadata).map_err(io_error)? {
                return Err(conflict("publication target changed identity before write verification"));
            }
            if let Some(identity) = source.expected_identity()
                && !publication_identity::matches_file(identity, &writable).map_err(io_error)?
            {
                return Err(conflict("rollback target changed file identity; retain concurrent file"));
            }
            if let Some(expected) = source.expected_bytes() {
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                let mut reader = directory.open_with(name, &options).map_err(io_error)?.into_std();
                if !matches_metadata(&reader, metadata).map_err(io_error)?
                    || !matches_bytes(&mut reader, expected).map_err(io_error)?
                {
                    return Err(conflict("publication target changed bytes; re-read before retry"));
                }
            }
            Some(writable)
        } else {
            if metadata.is_some() { return Err(conflict("new publication target already exists")); }
            None
        };
        before_replace();
        parent.verify()?;
        if !matches_named(&identity, directory, temporary).map_err(io_error)?
            || !matches_bytes(&mut staged, data).map_err(io_error)?
        {
            return Err(conflict("staged publication identity or bytes changed"));
        }
        if let Some(writable) = &writable {
            if !matches_metadata(writable, &directory.symlink_metadata(name).map_err(io_error)?).map_err(io_error)? {
                return Err(conflict("publication target changed identity before replacement"));
            }
            staged.set_permissions(writable.metadata().map_err(io_error)?.permissions()).map_err(io_error)?;
            #[cfg(target_os = "macos")]
            crate::service::copy_macos_publication_metadata(writable, &staged).map_err(|error| {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    AppError::Forbidden(format!("cannot preserve workspace publication ACL/xattrs: {error}"))
                } else { io_error(error) }
            })?;
            directory.rename(temporary, directory, name).map_err(io_error)?;
        } else {
            rename_noreplace(directory, temporary, name).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    conflict("new publication target appeared before commit")
                } else { io_error(error) }
            })?;
        }
        published = true;
        consumed = matches_named(&identity, directory, name).unwrap_or(false);
        if !consumed || !matches_bytes(&mut staged, data).map_err(io_error)? {
            return Err(AppError::Internal(format!("{}; published target identity or bytes were not confirmed",
                crate::service::FILE_WRITE_OUTCOME_UNKNOWN)));
        }
        verified = true;
        after_publication()?;
        parent.verify()?;
        sync_directory.sync_all().map_err(io_error)?;
        Ok(())
    })();
    result.map(|()| publication_identity.clone()).map_err(|error| PatchPublicationFailure {
        error, published, publication_verified: verified,
        publication_identity: if verified { publication_identity } else { None },
        // Do not unlink an unconsumed name through a non-atomic identity check.
        // Retain it for explicit reconciliation, as the existing Unix owner did.
        temporary_cleanup_unconfirmed: !consumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};

    #[cfg(target_os = "macos")]
    #[test]
    fn native_publication_and_directory_sync_require_only_parent_write_and_search() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let target = parent.join("target");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
        let result = crate::workspace_write::prepare_parent(&target, fixture.path())
            .map_err(PatchPublicationFailure::from)
            .and_then(|prepared| publish(&prepared, b"durable", PublicationSource::Absent));
        let listing_was_denied = fs::read_dir(&parent).is_err();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        result.unwrap();
        assert_eq!(fs::read(target).unwrap(), b"durable");
        if unsafe { libc::geteuid() } != 0 { assert!(listing_was_denied); }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_extended_attributes_survive_anchored_replacement() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        fs::write(&target, b"original").unwrap();
        let original = File::open(&target).unwrap();
        let name = CString::new("com.nomifun.anchored-publication-test").unwrap();
        let value = b"owned metadata";
        // SAFETY: the live fd, attribute name and sized buffer belong to this
        // temporary fixture; no ambient or user-file metadata is inspected.
        assert_eq!(unsafe { libc::fsetxattr(original.as_raw_fd(), name.as_ptr(),
            value.as_ptr().cast(), value.len(), 0, 0) }, 0);
        let parent = crate::workspace_write::prepare_parent(&target, fixture.path()).unwrap();
        publish(&parent, b"patched", PublicationSource::Matching(b"original")).unwrap();
        let replacement = File::open(&target).unwrap();
        let mut observed = [0u8; 64];
        let count = unsafe { libc::fgetxattr(replacement.as_raw_fd(), name.as_ptr(),
            observed.as_mut_ptr().cast(), observed.len(), 0, 0) };
        assert_eq!(count, value.len() as isize);
        assert_eq!(&observed[..value.len()], value);
        assert_eq!(fs::read(target).unwrap(), b"patched");
    }

    #[test]
    fn parent_replacement_cannot_redirect_new_existing_or_matching_publications() {
        for source in [PublicationSource::Absent, PublicationSource::Existing, PublicationSource::Matching(b"original")] {
            for after_staging in [false, true] {
                let fixture = tempfile::tempdir().unwrap();
                let root = fixture.path().join("workspace");
                let parent = root.join("parent");
                let retained = root.join("retained");
                let outside = fixture.path().join("outside");
                fs::create_dir_all(&parent).unwrap();
                fs::create_dir(&outside).unwrap();
                fs::write(outside.join("sentinel"), b"outside").unwrap();
                if !matches!(source, PublicationSource::Absent) {
                    fs::write(parent.join("target"), b"original").unwrap();
                    fs::write(outside.join("target"), b"original").unwrap();
                }
                let prepared = crate::workspace_write::prepare_parent(&parent.join("target"), &root).unwrap();
                let replace = || { fs::rename(&parent, &retained).unwrap(); symlink(&outside, &parent).unwrap(); };
                if !after_staging { replace(); }
                let result = publish_with_hooks(&prepared, b"patched", source, || if after_staging { replace(); }, || {}, || Ok(()));
                assert!(result.is_err());
                assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
                assert_eq!(fs::read_dir(&outside).unwrap().count(), if matches!(source, PublicationSource::Absent) { 1 } else { 2 });
                if !matches!(source, PublicationSource::Absent) {
                    assert_eq!(fs::read(outside.join("target")).unwrap(), b"original");
                    assert_eq!(fs::read(retained.join("target")).unwrap(), b"original");
                }
            }
        }
    }

    #[test]
    fn relative_publication_preserves_mode_and_rejects_read_only_or_concurrent_targets() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        fs::write(&target, b"original").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let parent = crate::workspace_write::prepare_parent(&target, fixture.path()).unwrap();
        publish(&parent, b"patched", PublicationSource::Matching(b"original")).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"patched");
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
        fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap();
        let result = publish(&parent, b"blocked", PublicationSource::Existing);
        if unsafe { libc::geteuid() } != 0 {
            assert!(matches!(result.unwrap_err().error, AppError::Forbidden(_)));
            assert_eq!(fs::read(&target).unwrap(), b"patched");
        }
        let new_target = fixture.path().join("new-target");
        let parent = crate::workspace_write::prepare_parent(&new_target, fixture.path()).unwrap();
        let result = publish_with_hooks(&parent, b"owned", PublicationSource::Absent, || {},
            || fs::write(&new_target, b"concurrent").unwrap(), || Ok(()));
        assert!(result.is_err());
        assert_eq!(fs::read(&new_target).unwrap(), b"concurrent");
    }
}
