//! Preserve Linux inode metadata before replacing an existing native file.
//! All operations use the verified source/staging descriptors. A failed copy
//! remains a pre-publication failure; OS ownership and xattr rules still apply.
use std::{collections::BTreeMap, ffi::CString, fs::File, io,
    os::{fd::AsRawFd, unix::fs::MetadataExt}};

// Linux's XATTR_SIZE_MAX and XATTR_LIST_MAX bound both native buffers.
const MAX_XATTR_BYTES: usize = 65_536;

type Attributes = BTreeMap<CString, Vec<u8>>;

fn attributes(file: &File) -> io::Result<Attributes> {
    let mut names = vec![0u8; MAX_XATTR_BYTES];
    // SAFETY: the fd and bounded writable buffer remain live for the call.
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
    if count < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOTSUP) { return Ok(BTreeMap::new()); }
        return Err(error);
    }
    names.truncate(count as usize);
    if !names.is_empty() && names.last() != Some(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "native xattr list is not terminated"));
    }
    let mut attributes = BTreeMap::new();
    for name in names.split_inclusive(|byte| *byte == 0) {
        let name = CString::from_vec_with_nul(name.to_vec())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut value = vec![0u8; MAX_XATTR_BYTES];
        // SAFETY: the live descriptor, terminated name and sized buffer are valid.
        let count = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        if count < 0 { return Err(io::Error::last_os_error()); }
        value.truncate(count as usize);
        attributes.insert(name, value);
    }
    Ok(attributes)
}

pub(crate) fn copy(source: &File, staged: &File) -> io::Result<()> {
    let source_metadata = source.metadata()?;
    let source_attributes = attributes(source)?;
    let staged_metadata = staged.metadata()?;
    if (source_metadata.uid(), source_metadata.gid()) != (staged_metadata.uid(), staged_metadata.gid()) {
        // Ownership may clear mode bits and file capabilities. Restore metadata
        // only afterwards, with every operation still subject to kernel checks.
        // SAFETY: the staging fd is live and uid/gid came from the verified source.
        if unsafe { libc::fchown(staged.as_raw_fd(), source_metadata.uid(), source_metadata.gid()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    staged.set_permissions(source_metadata.permissions())?;
    // A new inode may inherit a directory's default ACL or security attributes.
    // Keep the original file's actual metadata rather than those inherited extras.
    let staged_attributes = attributes(staged)?;
    for name in staged_attributes.keys().filter(|name| !source_attributes.contains_key(*name)) {
        // SAFETY: the live staging fd and terminated xattr name remain valid.
        if unsafe { libc::fremovexattr(staged.as_raw_fd(), name.as_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    for (name, value) in &source_attributes {
        if staged_attributes.get(name) == Some(value) { continue; }
        // Do not classify or bypass protected namespaces. The kernel decides
        // whether this caller may retain each attribute on the new inode.
        // SAFETY: the live fd, terminated name and sized value remain valid.
        if unsafe { libc::fsetxattr(staged.as_raw_fd(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let verified_staged_attributes = attributes(staged)?;
    let verified_source_attributes = attributes(source)?;
    let after = staged.metadata()?;
    let unchanged_source = source.metadata()?;
    if (after.uid(), after.gid(), after.mode()) != (source_metadata.uid(), source_metadata.gid(), source_metadata.mode())
        || (unchanged_source.uid(), unchanged_source.gid(), unchanged_source.mode(), unchanged_source.ctime(), unchanged_source.ctime_nsec())
            != (source_metadata.uid(), source_metadata.gid(), source_metadata.mode(), source_metadata.ctime(), source_metadata.ctime_nsec())
        || verified_staged_attributes != source_attributes || verified_source_attributes != source_attributes
    {
        return Err(io::Error::new(io::ErrorKind::Other, "publication metadata changed or could not be preserved"));
    }
    staged.sync_all()
}
