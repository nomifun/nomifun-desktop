//! nomifun-miniapp: backend for 小程序 (mini-apps).
//!
//! A mini-app is one self-contained single-file web tool the user solidified out
//! of a conversation (or imported): opening it later needs neither the
//! conversation nor its workspace.
//!
//! **Storage is two-layered on purpose.** The *published snapshot* stays in the
//! `miniapps.html` column and is what the serve route hands to an iframe; the
//! *working copy* lives on disk at
//! `{work_dir}/miniapps/{miniapp_id}/miniapp.html` and is what an editing
//! conversation rewrites in place. Serving a snapshot we wrote ourselves is what
//! makes a half-written document unservable: the working copy may be truncated
//! mid-write by any editor the agent happens to reach for, while `/serve` reads a
//! row. Crossing the two layers is one explicit act — [`MiniAppService::publish`]
//! — and until it happens the response's `has_unpublished_changes` says so.
//!
//! Two routers, deliberately split: [`miniapp_routes`] is the owner-scoped
//! management surface (list/create/read/update/delete/publish plus the workspace
//! provision, mounted under the instance-owner guard), and
//! [`miniapp_public_routes`] is the auth-exempt GET-only channel that hands the
//! stored document to an iframe. Responses on the management surface never carry
//! the HTML body — only its `html_size` — because the body has exactly one
//! consumer and it is the serve route.
//!
//! **This crate owns no conversation.** A thread that builds or edits a mini-app
//! is an ordinary conversation in an ordinary workspace; the only thing it is
//! given is the absolute path
//! [`MiniAppService::provision_workspace`] hands back, which it then reads and
//! writes with ordinary file tools. Nothing here creates, opens or deletes a
//! conversation, and no column points at one except the pure-provenance
//! `source_conversation_id`.

pub mod dto;
mod fsio;
pub mod import;
pub mod routes;
pub mod service;
pub mod state;
pub mod validation;

pub use dto::{
    CreateMiniAppRequest, MiniAppResponse, MiniAppWorkspaceResponse, UpdateMiniAppRequest,
};
// Re-exported so callers of this crate never have to spell the path formula
// themselves: it lives in `nomifun-common` because the tree is also the concern
// of the work-dir layout, not of this crate alone.
pub use nomifun_common::miniapp_workspace::{
    MINIAPP_SOURCE_FILE, MINIAPPS_REL_DIR, miniapp_workspace_dir, miniapps_root,
};
pub use routes::{miniapp_public_routes, miniapp_routes};
pub use service::{
    MiniAppService, MiniAppServiceError, MINI_APP_DESCRIPTION_MAX_CHARS, MINI_APP_HTML_MAX_BYTES,
    MINI_APP_ICON_MAX_CHARS, MINI_APP_NAME_MAX_CHARS,
};
pub use state::MiniAppRouterState;
