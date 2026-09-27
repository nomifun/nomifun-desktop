//! Publish a new file using the same handle that received its bytes.
use std::{fs::OpenOptions, io::Write, os::windows::fs::OpenOptionsExt, path::Path};
use nomifun_common::AppError;
use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_WRITE, FILE_SHARE_READ};

use crate::{agent_patch_outcome::PatchPublicationFailure, windows_cleanup::{remove_handle, rename_no_replace}};

pub(crate) fn publish(
    target: &Path, data: &[u8], temporary: &Path,
    after_staging: impl FnOnce(),
    after_publication: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), PatchPublicationFailure> {
    // Deny other writers and name changes from creation through publication.
    // DELETE is needed for both the rename and failure cleanup; neither uses
    // another path lookup or requires changing the workspace's permissions.
    let mut file = OpenOptions::new().create_new(true).write(true)
        .access_mode(FILE_GENERIC_WRITE | DELETE).share_mode(FILE_SHARE_READ)
        .open(temporary).map_err(|error| AppError::Internal(format!(
            "cannot create temporary patch file '{}': {error}", temporary.display()
        )))?;
    let mut published = false;
    let result = (|| {
        file.write_all(data).and_then(|()| file.sync_all()).map_err(|error| AppError::Internal(format!(
            "cannot stage new workspace file: {error}"
        )))?;
        after_staging();
        rename_no_replace(&file, target).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                AppError::Conflict(format!("patch target '{}' appeared during publication", target.display()))
            } else {
                AppError::Internal(format!("cannot publish new patch target '{}': {error}", target.display()))
            }
        })?;
        published = true;
        file.sync_all().map_err(|error| AppError::Internal(format!("cannot sync published workspace file: {error}")))
    })();
    let result = result.map_err(|error| PatchPublicationFailure {
        error,
        published,
        temporary_cleanup_unconfirmed: !published && remove_handle(&file).is_err(),
    });
    drop(file);
    result?;
    after_publication().map_err(|error| PatchPublicationFailure {
        error, published: true, temporary_cleanup_unconfirmed: false,
    })
}
