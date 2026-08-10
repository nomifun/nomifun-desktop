//! nomifun-miniapp: backend for 小程序 (mini-apps).
//!
//! A mini-app is one AI-generated, self-contained single-file web tool that a
//! conversation produced and the user solidified: the whole HTML document is
//! stored in the `miniapps` table, so opening it later needs neither the
//! conversation nor its workspace.
//!
//! Two routers, deliberately split: [`miniapp_routes`] is the owner-scoped
//! management surface (list/create/read/update/delete, mounted under the
//! instance-owner guard), and [`miniapp_public_routes`] is the auth-exempt
//! GET-only channel that hands the stored document to an iframe. Responses on
//! the management surface never carry the HTML body — only its `html_size` —
//! because the body has exactly one consumer and it is the serve route.

pub mod dto;
pub mod routes;
pub mod service;
pub mod state;

pub use dto::{CreateMiniAppRequest, MiniAppResponse, UpdateMiniAppRequest};
pub use routes::{miniapp_public_routes, miniapp_routes};
pub use service::{
    MiniAppService, MiniAppServiceError, MINI_APP_DESCRIPTION_MAX_CHARS, MINI_APP_HTML_MAX_BYTES,
    MINI_APP_ICON_MAX_CHARS, MINI_APP_NAME_MAX_CHARS,
};
pub use state::MiniAppRouterState;
