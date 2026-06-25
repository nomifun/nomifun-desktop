//! OS shell integration: file/folder opener, tool detection, and speech-to-text.
pub mod error;
pub mod opener;
pub mod routes;
pub mod shell;
pub mod state;
pub mod stt;
pub(crate) mod stt_deepgram;
pub(crate) mod stt_openai;

pub use error::{ShellError, SttError};
pub use opener::{DefaultSystemOpener, ISystemOpener, NoopSystemOpener};
pub use routes::shell_routes;
pub use shell::ShellService;
pub use state::ShellRouterState;
pub use stt::SttService;
