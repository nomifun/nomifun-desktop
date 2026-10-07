//! Keep the created object alive until a multi-file patch finishes compensation.
use std::{fs::File, io, path::Path, sync::Arc};

#[cfg(windows)]
pub(crate) type PublicationIdentity = Arc<crate::windows_cleanup::OwnedFile>;
#[cfg(not(windows))]
pub(crate) type PublicationIdentity = Arc<File>;

pub(crate) fn capture(file: &File) -> io::Result<PublicationIdentity> {
    #[cfg(windows)]
    { crate::windows_cleanup::OwnedFile::capture(file).map(Arc::new) }
    #[cfg(not(windows))]
    { file.try_clone().map(Arc::new) }
}

pub(crate) fn matches_file(identity: &PublicationIdentity, current: &File) -> io::Result<bool> {
    #[cfg(windows)]
    { identity.matches_handle(current) }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected = identity.metadata()?;
        let actual = current.metadata()?;
        Ok(actual.is_file() && expected.dev() == actual.dev() && expected.ino() == actual.ino())
    }
    #[cfg(not(any(windows, unix)))]
    { let _ = (identity, current); Err(io::Error::new(io::ErrorKind::Unsupported, "file identity is unavailable")) }
}

pub(crate) fn matches_path(identity: &PublicationIdentity, path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    { identity.verify_named_identity(path).map(|()| true) }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected = identity.metadata()?;
        let actual = std::fs::symlink_metadata(path)?;
        Ok(actual.is_file() && expected.dev() == actual.dev() && expected.ino() == actual.ino())
    }
    #[cfg(not(any(windows, unix)))]
    { let _ = (identity, path); Err(io::Error::new(io::ErrorKind::Unsupported, "file identity is unavailable")) }
}
