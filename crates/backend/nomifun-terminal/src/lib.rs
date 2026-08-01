//! Terminal sessions: PTY-backed interactive sessions managed alongside
//! conversations. PTY process lifetimes are owned by `nomi-process-runtime`,
//! output is streamed over the realtime WebSocket broadcaster, and session
//! metadata is persisted in SQLite.

/// ANSI/OSC escape stripping + incremental line scanning. The implementation
/// lives in `nomifun_common::ansi`; this alias keeps `nomifun_terminal::ansi`
/// paths and the crate-root re-exports working for existing consumers
/// (e.g. `nomifun-idmm`).
pub mod ansi {
    pub use nomifun_common::ansi::*;
}
pub mod driver;
pub mod enhance;
pub mod error;
pub mod events;
pub mod lifecycle;
pub mod pty;
pub mod routes;
pub mod service;
pub mod state;
pub mod submit;
pub mod title;
pub mod types;

pub use ansi::{AnsiLineScanner, strip_ansi};
pub use driver::{TerminalDescription, TerminalDriver};
pub use enhance::{apply_enhancement, resolve_agent_family, terminal_autowork_capable, AgentCli, LifecycleHookWiring, McpServerSpec, TerminalLaunchEnhancement};
pub use events::TerminalEventEmitter;
pub use lifecycle::{
    ExactTerminalLifecycleReceiver, LifecycleKind, TerminalLifecycleEvent,
    TerminalLifecycleServer,
};
pub use routes::terminal_routes;
pub use service::{TerminalService, TerminalSupervisionHook, TerminalOutputTail};
pub use state::{TerminalConversationNoticeSink, TerminalRouterState};
pub use submit::{encode_submit_chunks, SubmitChunks, SettleReason, IDLE_SETTLE_WINDOW, TERMINAL_SUBMIT_DELAY};
pub use title::{TerminalTitleCompleter, clamp_title, fallback_title, TITLE_MAX_CHARS};
