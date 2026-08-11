//! The one formula that turns a mini-app id into its on-disk workspace.
//!
//! Lives here, not in `nomifun-miniapp`, because the tree is part of the work-dir
//! layout this crate defines: `nomifun-miniapp` creates the directory and
//! materializes the working copy into it, and the escape guard on that side is
//! defence in depth over this formula.
//!
//! The path is a pure function of the id — there is deliberately no path column.
//! A stored absolute path goes stale the moment the user relocates their work
//! dir, which is why `bootstrap/relocation.rs` has to rewrite a hand-maintained
//! list of text columns; a derived path needs no entry in it.
//!
//! Note the asymmetry this encodes: the *artifact* has a dedicated path, the
//! *conversation* that edits it does not. A conversation that improves a mini-app
//! is an ordinary conversation in an ordinary workspace which was told this
//! absolute path; nothing redirects its cwd here, so deleting that conversation
//! cannot touch the app.
//!
//! `{work_dir}` and not `{data_dir}`: these are user-authored working files an
//! agent reads and writes every turn — the same category as `conversations/`.
//! The tree is deliberately NOT registered in `MANAGED_DATASET_ROOTS`: that
//! registry is frozen per released reset-plan version, so a new entry would have
//! to mint `RELEASED_V3_MANAGED_ROOTS` and bump `PLAN_VERSION` (the
//! `browser-secrets` regression is the documented cost of editing it in place).
//! What that trades away is spelled out in the v2 spec: a factory reset does not
//! sweep this tree and an offline backup does not carry it — but the runnable
//! artifact, the published snapshot in `miniapps.html`, is in the database and
//! therefore in every backup.

use std::path::{Path, PathBuf};

/// Directory below `{work_dir}` that holds one subdirectory per mini-app.
///
/// Spelled `miniapps` (no dash) to match the table, the id type and the route
/// prefix; the frontend contract constant for the file inside it is
/// `MINI_APP_FILE_NAME` in `ui/src/renderer/pages/miniApps/contract.ts`.
pub const MINIAPPS_REL_DIR: &str = "miniapps";

/// The working copy an editing conversation rewrites in place.
pub const MINIAPP_SOURCE_FILE: &str = "miniapp.html";

/// `{work_dir}/miniapps` — the parent of every mini-app workspace.
pub fn miniapps_root(work_dir: &Path) -> PathBuf {
    work_dir.join(MINIAPPS_REL_DIR)
}

/// `{work_dir}/miniapps/{miniapp_id}` — one mini-app's private workspace.
///
/// `miniapp_id` must already be a validated bare UUIDv7 (`MiniAppId`). Callers
/// that hold only a string must parse it first: this function joins whatever it
/// is given, and the escape guard on the read/write side is defence in depth,
/// not the primary check.
pub fn miniapp_workspace_dir(work_dir: &Path, miniapp_id: &str) -> PathBuf {
    miniapps_root(work_dir).join(miniapp_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_is_a_pure_function_of_the_id() {
        let work = Path::new("/w");
        let id = "0190f5fe-7c00-7a00-8000-000000000001";
        assert_eq!(
            miniapp_workspace_dir(work, id),
            Path::new("/w").join("miniapps").join(id)
        );
        // Same inputs, same answer: nothing is read from a column or a cache, so
        // relocating the work dir moves every mini-app with it.
        assert_eq!(
            miniapp_workspace_dir(work, id),
            miniapp_workspace_dir(work, id)
        );
        assert!(miniapp_workspace_dir(work, id).starts_with(miniapps_root(work)));
    }
}
