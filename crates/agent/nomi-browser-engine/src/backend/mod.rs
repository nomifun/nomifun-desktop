//! Browser backends. P0 ships a single [`cdp::CdpBackend`] (方案 A：自建 transport 发裸
//! CDP 命令，**不**用 chromiumoxide 高层 `Browser`/`Page`)。

pub mod cdp;

pub use cdp::CdpBackend;
