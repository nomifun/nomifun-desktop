//! Confined Windows removal without reopening checked entries by path.
use std::{fs::{File, OpenOptions}, io, path::Path};
use std::os::windows::{fs::{OpenOptionsExt, MetadataExt}, io::AsRawHandle};
use cap_std::fs::Dir;
use nomifun_common::AppError;
use windows_sys::Win32::{Storage::FileSystem::{
    DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_ATTRIBUTE_REPARSE_POINT,
}};

pub(crate) fn remove_entry(path: &Path, root: Option<&crate::windows_read::ReadRoot>, after_open: impl FnOnce()) -> Result<(), AppError> {
    let io_error = |error: io::Error| crate::path_safety::native_io_error(format!("removing native entry '{}'", path.display()), error);
    let file = match root {
        Some(root) => root.open_for_delete(path)?,
        None => OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(path).map_err(io_error)?,
    };
    if root.is_some() && file.metadata().map_err(io_error)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(AppError::Conflict("workspace deletion target changed to a link".into()));
    }
    remove_pinned(path, file, after_open)
}

pub(crate) fn remove_pinned(path: &Path, file: File, after_open: impl FnOnce()) -> Result<(), AppError> {
    let io_error = |error: io::Error| crate::path_safety::native_io_error(format!("removing native entry '{}'", path.display()), error);
    let metadata = file.metadata().map_err(io_error)?;
    let is_link = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    if !is_link && !metadata.is_dir() && !metadata.is_file() {
        return Err(AppError::Conflict("workspace deletion target changed to a link or unsupported entry".into()));
    }
    if crate::windows_read::final_path(&file)? != path {
        return Err(AppError::Conflict("native deletion target changed identity before open".into()));
    }
    let name = crate::windows_cleanup::DeletionName::capture(&file).map_err(io_error)?;
    after_open();
    if metadata.is_dir() && !is_link {
        remove_tree(file, name).map_err(|error| AppError::Internal(format!(
            "{}; recursive removal of '{}' failed: {error}; inspect the remaining tree before retry",
            crate::service::FILE_DELETE_OUTCOME_UNKNOWN, path.display()
        )))
    } else {
        // Preserve the ordinary OS error if nothing was marked. Once marked,
        // a still-present/delete-pending name is an uncertain effect.
        crate::windows_cleanup::mark_delete(&file, false).map_err(io_error)?;
        drop(file);
        name.verify_absent().map_err(|error| AppError::Internal(format!(
            "{}; cleanup {}; deletion of '{}' is unconfirmed; inspect before retry",
            crate::service::FILE_DELETE_OUTCOME_UNKNOWN, crate::windows_cleanup::error_summary(&error), path.display()
        )))
    }
}

fn unlink(file: &impl AsRawHandle) -> io::Result<()> {
    // No readonly override for a canonical user-requested deletion.
    crate::windows_cleanup::mark_delete(file, false)
}

struct Frame { directory: Dir, entries: crate::windows_directory::Entries, name: crate::windows_cleanup::DeletionName }

impl Frame {
    fn new(file: File, name: crate::windows_cleanup::DeletionName) -> io::Result<Self> {
        let entries = crate::windows_directory::Entries::new(crate::windows_directory::open_cursor(&file)?);
        Ok(Self { directory: Dir::from_std_file(file), entries, name })
    }
}

fn remove_tree(file: File, name: crate::windows_cleanup::DeletionName) -> io::Result<()> {
    // Empty-directory deletion does not require LIST_DIRECTORY access.
    match unlink(&file) {
        Ok(()) => { drop(file); return name.verify_absent(); },
        Err(error) if error.raw_os_error() == Some(145) => {},
        Err(error) => return Err(error),
    }
    // Explicit frames avoid exhausting the call stack on deep native paths.
    let mut frames = vec![Frame::new(file, name)?];
    while let Some(frame) = frames.last_mut() {
        match frame.entries.next() {
            Some(name) => {
                let child = crate::windows_directory::open_delete(Some(&frame.directory), Path::new(&name?.name))?;
                let child_name = crate::windows_cleanup::DeletionName::capture(&child)?;
                let metadata = child.metadata()?;
                // Reparse entries are unlinked themselves, never traversed.
                if metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    match unlink(&child) {
                        Ok(()) => { drop(child); child_name.verify_absent()?; },
                        Err(error) if error.raw_os_error() == Some(145) => frames.push(Frame::new(child, child_name)?),
                        Err(error) => return Err(error),
                    }
                } else {
                    unlink(&child)?;
                    drop(child);
                    child_name.verify_absent()?;
                }
            }
            None => {
                let frame = frames.pop().expect("a directory frame remains");
                drop(frame.entries);
                unlink(&frame.directory)?;
                drop(frame.directory);
                frame.name.verify_absent()?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;

    #[test]
    fn basic_single_file_delete_is_unknown_while_an_external_reader_holds_it() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().canonicalize().unwrap().join("held");
        fs::write(&path, b"fixture").unwrap();
        let reader = OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(&path).unwrap();
        let error = crate::windows_cleanup::with_basic_disposition(|| remove_entry(&path, None, || {})).unwrap_err();
        assert!(crate::service::file_delete_outcome_unknown(&error), "{error}");
        assert!(error.to_string().contains("cleanup raw_os_error="));
        drop(reader);
        crate::windows_cleanup::verify_absent(&path).unwrap();
    }

    #[test]
    fn basic_delete_proof_keeps_the_original_parent_after_its_name_is_replaced() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let parent = root.join("parent");
        let moved = root.join("moved");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("held");
        fs::write(&path, b"fixture").unwrap();
        let initial = fs::File::open(&path).unwrap();
        let name = crate::windows_cleanup::DeletionName::capture(&initial).unwrap();
        drop(initial);
        // Local NTFS refuses moving this parent *after* a child is marked
        // delete-pending. Relocate before the mark to independently test the
        // retained namespace proof, without claiming that refused race ran.
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();
        let moved_path = moved.join("held");
        let reader = OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(&moved_path).unwrap();
        let deleting = crate::windows_directory::open_delete(None, &moved_path).unwrap();
        crate::windows_cleanup::with_basic_disposition(|| crate::windows_cleanup::mark_delete(&deleting, false)).unwrap();
        drop(deleting);
        assert_eq!(fs::symlink_metadata(&path).unwrap_err().raw_os_error(), Some(2),
            "ambient lookup misleadingly sees a missing name in a replacement parent");
        let error = name.verify_absent().unwrap_err();
        assert!(!matches!(error.raw_os_error(), Some(2 | 3)), "{error}");
        drop(reader);
        name.verify_absent().unwrap();
        crate::windows_cleanup::verify_absent(&moved_path).unwrap();
        assert!(parent.is_dir() && moved.is_dir());
    }

    #[test]
    fn basic_single_file_delete_keeps_pre_mutation_readonly_failure_certain() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().canonicalize().unwrap().join("readonly");
        fs::write(&path, b"fixture").unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original.clone(); readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let error = crate::windows_cleanup::with_basic_disposition(|| remove_entry(&path, None, || {})).unwrap_err();
        let still_readonly = fs::metadata(&path).unwrap().permissions().readonly();
        fs::set_permissions(&path, original).unwrap();
        assert!(!crate::service::file_delete_outcome_unknown(&error), "{error}");
        assert!(still_readonly);
        assert_eq!(fs::read(&path).unwrap(), b"fixture");
    }

    #[test]
    fn basic_recursive_delete_closes_own_cursors_and_unlinks_junction_only() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let target = root.join("tree");
        let outside = root.join("outside");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(target.join("nested/owned"), b"owned").unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        junction::create(&outside, target.join("link")).unwrap();
        crate::windows_cleanup::with_basic_disposition(|| remove_entry(&target, None, || {})).unwrap();
        crate::windows_cleanup::verify_absent(&target).unwrap();
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    }
}
