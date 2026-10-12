//! Descriptor-confined ZIP sources, verified again before archive publication.

use std::fs::{File, Metadata};
use std::path::PathBuf;

use nomifun_common::AppError;
use same_file::Handle;

use crate::path_safety::{PathAuthority, validate_path_authority};

pub(crate) struct ZipSource {
    pub(crate) file: File,
    pub(crate) length: u64,
    metadata: Metadata,
    identity: Handle,
    canonical: PathBuf,
    #[cfg(windows)]
    root: Option<crate::windows_read::ReadRoot>,
    #[cfg(not(windows))]
    root: cap_std::fs::Dir,
    #[cfg(not(windows))]
    root_path: PathBuf,
    #[cfg(not(windows))]
    root_identity: Handle,
    #[cfg(not(windows))]
    relative: PathBuf,
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::BadRequest(format!("cannot read ZIP source: {error}"))
}

fn changed() -> AppError {
    AppError::Conflict("ZIP source changed during export; retry with a fresh workspace inventory".into())
}

impl ZipSource {
    pub(crate) fn open(path: &str, authority: &PathAuthority, before_open: impl FnOnce()) -> Result<Self, AppError> {
        let canonical = validate_path_authority(path, authority)?;
        if !std::fs::symlink_metadata(&canonical).map_err(io_error)?.is_file() {
            return Err(AppError::BadRequest("ZIP source must be a regular file".into()));
        }
        #[cfg(windows)]
        let root = crate::windows_read::ReadRoot::for_target(&canonical, authority)?;
        #[cfg(not(windows))]
        let (root, root_path, root_identity, relative) = {
            let roots = match authority {
                PathAuthority::Unrestricted => vec![canonical.parent().ok_or_else(changed)?.to_path_buf()],
                PathAuthority::Confined(roots) => roots.clone(),
                PathAuthority::Workspace(root) => vec![root.clone()],
            };
            let root_path = roots.into_iter().filter_map(|root| std::fs::canonicalize(root).ok())
                .filter(|root| canonical.starts_with(root))
                .max_by_key(|root| root.components().count()).ok_or_else(changed)?;
            let relative = canonical.strip_prefix(&root_path).map_err(|_| changed())?.to_path_buf();
            let root = cap_std::fs::Dir::open_ambient_dir(&root_path, cap_std::ambient_authority()).map_err(io_error)?;
            let root_identity = Handle::from_file(root.try_clone().map_err(io_error)?.into_std_file()).map_err(io_error)?;
            (root, root_path, root_identity, relative)
        };
        before_open();
        #[cfg(windows)]
        let file = match &root {
            Some(root) => root.open(&canonical)?,
            None => File::open(&canonical).map_err(io_error)?,
        };
        #[cfg(not(windows))]
        let file = root.open_with(&relative, &read_options()).map_err(io_error)?.into_std();
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() {
            return Err(AppError::BadRequest("ZIP source must be a regular file".into()));
        }
        let identity = Handle::from_file(file.try_clone().map_err(io_error)?).map_err(io_error)?;
        let source = Self {
            length: metadata.len(), file, metadata, identity, canonical, root,
            #[cfg(not(windows))] root_path,
            #[cfg(not(windows))] root_identity,
            #[cfg(not(windows))] relative,
        };
        source.verify(source.length, authority)?;
        Ok(source)
    }

    pub(crate) fn verify(&self, copied_bytes: u64, authority: &PathAuthority) -> Result<(), AppError> {
        let after = self.file.metadata().map_err(io_error)?;
        if copied_bytes != self.length || after.len() != self.length
            || after.modified().ok() != self.metadata.modified().ok()
            || validate_path_authority(&self.canonical.to_string_lossy(), authority).map_err(|_| changed())? != self.canonical
        {
            return Err(changed());
        }
        #[cfg(windows)]
        let current = match &self.root {
            Some(root) => { root.verify()?; root.open(&self.canonical)? },
            None => File::open(&self.canonical).map_err(|_| changed())?,
        };
        #[cfg(not(windows))]
        let current = {
            let named_root = cap_std::fs::Dir::open_ambient_dir(&self.root_path, cap_std::ambient_authority())
                .map_err(|_| changed())?;
            if Handle::from_file(named_root.into_std_file()).map_err(io_error)? != self.root_identity
                || self.root.canonicalize(&self.relative).map_err(|_| changed())? != self.relative
            {
                return Err(changed());
            }
            self.root.open_with(&self.relative, &read_options()).map_err(|_| changed())?.into_std()
        };
        if Handle::from_file(current).map_err(io_error)? != self.identity {
            return Err(changed());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn read_options() -> cap_std::fs::OpenOptions {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt as _;
        // A concurrently substituted FIFO must not block an export worker.
        options.custom_flags(libc::O_NONBLOCK);
    }
    options
}
