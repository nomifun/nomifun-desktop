use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nomifun_v4_root::{
    FRESH_V4_INITIALIZING_MARKER_FILE, FRESH_V4_PARENT_MARKER_FILE,
    FRESH_V4_READY_MARKER_FILE, FreshV4BootstrapOutcome, FreshV4Coordinator,
};

const APPLICATION_BUILD_IDENTITY: &str =
    concat!("nomifun-app@", env!("CARGO_PKG_VERSION"));

pub(super) fn bootstrap_data_root(
    data_root: &Path,
) -> Result<FreshV4BootstrapOutcome> {
    run_coordinator(data_root).with_context(|| {
        format!(
            "Fresh-v4 pre-service bootstrap failed for {}",
            data_root.display()
        )
    })
}

pub(super) fn recover_or_validate_if_present(data_root: &Path) -> Result<()> {
    if has_v4_evidence(data_root)? {
        bootstrap_data_root(data_root)?;
    }
    Ok(())
}

pub(super) fn reject_legacy_v3_data_layer(
    data_root: &Path,
    operation: &str,
) -> Result<()> {
    recover_or_validate_if_present(data_root)?;
    if path_present(&data_root.join(FRESH_V4_READY_MARKER_FILE))? {
        anyhow::bail!(
            "{operation} is fenced from the ready Fresh-v4 root at {}; \
             the v4 service composition must own this database",
            data_root.display()
        );
    }
    if parent_marker_present(data_root)? {
        anyhow::bail!(
            "{operation} is fenced while a Fresh-v4 parent operation marker is active for {}",
            data_root.display()
        );
    }
    Ok(())
}

fn run_coordinator(
    data_root: &Path,
) -> Result<FreshV4BootstrapOutcome> {
    let data_root = data_root.to_path_buf();
    let protected_roots = vec![
        std::env::current_dir()
            .context("resolve current directory for Fresh-v4 root protection")?,
    ];
    std::thread::Builder::new()
        .name("nomifun-fresh-v4-bootstrap".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build Fresh-v4 bootstrap runtime")?;
            runtime
                .block_on(
                    FreshV4Coordinator::default().bootstrap(
                        &data_root,
                        APPLICATION_BUILD_IDENTITY,
                        &protected_roots,
                    ),
                )
                .map_err(anyhow::Error::from)
        })
        .context("spawn Fresh-v4 bootstrap thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("Fresh-v4 bootstrap thread panicked"))?
}

fn has_v4_evidence(data_root: &Path) -> Result<bool> {
    Ok(path_present(&data_root.join(FRESH_V4_READY_MARKER_FILE))?
        || path_present(
            &data_root.join(FRESH_V4_INITIALIZING_MARKER_FILE),
        )?
        || parent_marker_present(data_root)?)
}

fn parent_marker_path(data_root: &Path) -> Option<PathBuf> {
    data_root
        .parent()
        .map(|parent| parent.join(FRESH_V4_PARENT_MARKER_FILE))
}

fn parent_marker_present(data_root: &Path) -> Result<bool> {
    parent_marker_path(data_root)
        .map(|path| path_present(&path))
        .unwrap_or(Ok(false))
}

fn path_present(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("inspect Fresh-v4 evidence {}", path.display())),
    }
}
