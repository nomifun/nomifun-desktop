//! A small file transaction for the application files supplied by an installer.
//! It never enumerates or copies the user's data tree. Only payload-relative
//! paths can be touched, and each pre-existing file is backed up before writes.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

fn regular_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "installer path is a symbolic link: {}", path.display());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(metadata.file_attributes() & 0x400 == 0, "installer path is a reparse point: {}", path.display());
    }
    Ok(())
}

fn within(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(relative.components().all(|part| matches!(part, Component::Normal(_))), "invalid installer relative path");
    let mut path = root.to_path_buf();
    regular_path(&path)?;
    for component in relative.components() {
        path.push(component);
        if path.exists() {
            regular_path(&path)?;
        } else {
            // symlink_metadata also finds dangling links, unlike exists().
            ensure!(fs::symlink_metadata(&path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound), "unsafe installer target");
        }
    }
    Ok(path)
}

fn equal(a: &Path, b: &Path) -> Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let mut a = File::open(a)?;
    let mut b = File::open(b)?;
    let mut left = [0; 64 * 1024];
    let mut right = [0; 64 * 1024];
    loop {
        let count = a.read(&mut left)?;
        b.read_exact(&mut right[..count])?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
        if count == 0 {
            return Ok(true);
        }
    }
}

fn collect(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    regular_path(directory)?;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        regular_path(&path)?;
        if path.is_dir() {
            collect(root, &path, paths)?;
        } else {
            ensure!(path.is_file(), "installer payload contains a special file");
            paths.push(path.strip_prefix(root)?.to_path_buf());
        }
    }
    Ok(())
}

struct Entry {
    relative: PathBuf,
    existed: bool,
    touched: bool,
}

pub struct FileReplacement {
    payload: PathBuf,
    destination: PathBuf,
    backup: PathBuf,
    entries: Vec<Entry>,
    created_directories: Vec<PathBuf>,
    committed: bool,
    rollback_attempted: bool,
}

impl FileReplacement {
    pub fn prepare(payload: &Path, destination: &Path, backup: &Path) -> Result<Self> {
        ensure!(payload.is_absolute() && destination.is_absolute() && backup.is_absolute(), "installer paths must be absolute");
        regular_path(payload)?;
        ensure!(payload.is_dir(), "installer payload is not a directory");
        ensure!(!backup.exists(), "installer backup already exists");
        let mut paths = Vec::new();
        collect(payload, payload, &mut paths)?;
        paths.sort();
        ensure!(!paths.is_empty(), "installer payload is empty");
        // The installation root is the only directory the package may create
        // outside its private session. Its parent must already exist.
        if !destination.exists() {
            let parent = destination.parent().context("installation root has no parent")?;
            ensure!(parent.is_dir(), "installation parent is missing");
            fs::create_dir(destination)?;
        }
        regular_path(destination)?;
        ensure!(destination.is_dir(), "installation root is not a directory");
        let payload = payload.canonicalize()?;
        let destination = destination.canonicalize()?;
        ensure!(!payload.starts_with(&destination) && !destination.starts_with(&payload), "payload and installation must be separate");
        fs::create_dir(backup)?;
        regular_path(backup)?;
        let backup = backup.canonicalize()?;
        ensure!(!backup.starts_with(&destination), "backup must be outside the installation");
        let mut entries = Vec::new();
        // Snapshot every affected original before changing any application file.
        for relative in paths {
            let target = within(&destination, &relative)?;
            let existed = target.exists();
            if existed {
                ensure!(target.is_file(), "payload file would replace a directory: {}", target.display());
                let saved = backup.join(&relative);
                fs::create_dir_all(saved.parent().context("backup has no parent")?)?;
                fs::copy(&target, &saved).with_context(|| format!("back up {}", target.display()))?;
                ensure!(equal(&target, &saved)?, "application changed during backup");
            }
            entries.push(Entry { relative, existed, touched: false });
        }
        Ok(Self { payload, destination, backup, entries, created_directories: Vec::new(), committed: false, rollback_attempted: false })
    }

    pub fn apply(&mut self) -> Result<()> {
        ensure!(!self.committed, "application replacement is already committed");
        self.rollback_attempted = false;
        for index in 0..self.entries.len() {
            let target = within(&self.destination, &self.entries[index].relative)?;
            self.create_parents(target.parent().context("replacement has no parent")?)?;
            self.entries[index].touched = true;
            Self::replace(&self.payload.join(&self.entries[index].relative), &target)
                .with_context(|| format!("replace {}", target.display()))?;
        }
        Ok(())
    }

    fn create_parents(&mut self, parent: &Path) -> Result<()> {
        if parent.exists() {
            regular_path(parent)?;
            return Ok(());
        }
        self.create_parents(parent.parent().context("replacement parent is missing")?)?;
        fs::create_dir(parent)?;
        self.created_directories.push(parent.into());
        Ok(())
    }

    fn replace(source: &Path, target: &Path) -> Result<()> {
        regular_path(source)?;
        let name = target.file_name().context("replacement target has no name")?;
        let temporary = target.with_file_name(format!(".{}.nomifun-{}", name.to_string_lossy(), uuid::Uuid::now_v7()));
        let result = (|| {
            let mut input = File::open(source)?;
            let mut output = File::options().write(true).create_new(true).open(&temporary)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            drop(output);
            fs::set_permissions(&temporary, fs::metadata(source)?.permissions())?;
            ensure!(equal(source, &temporary)?, "staged application copy differs from payload");
            fs::rename(&temporary, target)?;
            ensure!(equal(source, target)?, "installed application copy differs from payload");
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    /// Returns success only after independently comparing every restored file.
    /// A sharing error on a file that never changed does not prevent rollback.
    pub fn rollback(&mut self) -> Result<()> {
        ensure!(!self.committed, "committed application replacement cannot be rolled back");
        self.rollback_attempted = true;
        let mut errors = Vec::new();
        for entry in self.entries.iter_mut().rev().filter(|entry| entry.touched) {
            let result = (|| -> Result<()> {
                let target = within(&self.destination, &entry.relative)?;
                if entry.existed {
                    let original = self.backup.join(&entry.relative);
                    if !target.is_file() || !equal(&original, &target)? {
                        Self::replace(&original, &target)?;
                    }
                    ensure!(equal(&original, &target)?, "restored application file differs from backup");
                } else if target.exists() {
                    fs::remove_file(&target)?;
                    ensure!(!target.exists(), "new application file survived rollback");
                }
                entry.touched = false;
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!("{}: {error:#}", entry.relative.display()));
            }
        }
        for directory in self.created_directories.iter().rev() {
            if directory.exists() {
                let _ = fs::remove_dir(directory); // only empty directories created by this transaction
            }
        }
        ensure!(errors.is_empty(), "application rollback requires repair: {}", errors.join("; "));
        Ok(())
    }

    pub fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for FileReplacement {
    fn drop(&mut self) {
        if !self.committed && !self.rollback_attempted {
            let _ = self.rollback();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let payload = dir.path().join("payload");
        let installed = dir.path().join("installed");
        let backup = dir.path().join("backup");
        fs::create_dir(&payload).unwrap();
        fs::create_dir(&installed).unwrap();
        fs::write(installed.join("app.exe"), b"old app").unwrap();
        fs::write(installed.join("settings.json"), b"user preference").unwrap();
        fs::write(payload.join("app.exe"), b"new app").unwrap();
        fs::create_dir(payload.join("resources")).unwrap();
        fs::write(payload.join("resources/new.dat"), b"new resource").unwrap();
        (dir, payload, installed, backup)
    }

    #[test]
    fn successful_replacement_preserves_files_outside_payload() {
        let (_dir, payload, installed, backup) = fixture();
        let mut transaction = FileReplacement::prepare(&payload, &installed, &backup).unwrap();
        transaction.apply().unwrap();
        transaction.commit();
        drop(transaction);
        assert_eq!(fs::read(installed.join("app.exe")).unwrap(), b"new app");
        assert_eq!(fs::read(installed.join("resources/new.dat")).unwrap(), b"new resource");
        assert_eq!(fs::read(installed.join("settings.json")).unwrap(), b"user preference");
    }

    #[test]
    fn cancellation_and_drop_restore_original_and_remove_new_files() {
        for explicit in [true, false] {
            let (_dir, payload, installed, backup) = fixture();
            let mut transaction = FileReplacement::prepare(&payload, &installed, &backup).unwrap();
            transaction.apply().unwrap();
            if explicit { transaction.rollback().unwrap(); }
            drop(transaction);
            assert_eq!(fs::read(installed.join("app.exe")).unwrap(), b"old app");
            assert!(!installed.join("resources/new.dat").exists());
            assert_eq!(fs::read(installed.join("settings.json")).unwrap(), b"user preference");
        }
    }

    #[test]
    fn failed_midway_replacement_restores_previous_files() {
        let (_dir, payload, installed, backup) = fixture();
        let mut transaction = FileReplacement::prepare(&payload, &installed, &backup).unwrap();
        // The second file disappearing models an I/O fault after the first copy.
        fs::remove_file(payload.join("resources/new.dat")).unwrap();
        assert!(transaction.apply().is_err());
        transaction.rollback().unwrap();
        assert_eq!(fs::read(installed.join("app.exe")).unwrap(), b"old app");
        assert!(!installed.join("resources/new.dat").exists());
    }

    #[test]
    fn refuses_paths_that_cross_a_symlink() {
        let (_dir, payload, installed, backup) = fixture();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&payload, installed.join("resources")).unwrap();
        #[cfg(windows)]
        {
            // Creating Windows symlinks requires Developer Mode or privilege;
            // an ordinary file/directory collision exercises the same refusal.
            fs::create_dir(installed.join("app.exe.dir")).unwrap();
            fs::rename(installed.join("app.exe"), installed.join("saved.exe")).unwrap();
            fs::rename(installed.join("app.exe.dir"), installed.join("app.exe")).unwrap();
        }
        assert!(FileReplacement::prepare(&payload, &installed, &backup).is_err());
    }
}
