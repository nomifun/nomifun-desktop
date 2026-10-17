//! Only the approved Office renderer is embedded. Downloaded tools share the manifest.
use crate::ToolId;
mod bun {
    include!(concat!(env!("OUT_DIR"), "/bun_identity.rs"));
}
mod node {
    include!(concat!(env!("OUT_DIR"), "/node_identity.rs"));
}
mod officecli {
    include!(concat!(env!("OUT_DIR"), "/officecli_meta.rs"));
}

#[derive(Clone, Copy)]
pub(crate) struct EmbeddedTool {
    pub blob: &'static [u8],
    pub sha256: &'static str,
    pub version: &'static str,
    pub executable: &'static str,
}

pub(crate) fn embedded(id: ToolId) -> Option<EmbeddedTool> {
    (id == ToolId::OfficeCli && officecli::HAS_EMBEDDED).then_some(EmbeddedTool {
        blob: officecli::BLOB,
        sha256: officecli::SHA256,
        version: officecli::VERSION,
        executable: officecli::EXECUTABLE,
    })
}

#[derive(Clone, Copy)]
pub(crate) struct ExecutableIdentity {
    pub distribution_sha256: Option<&'static str>,
    pub sha256: &'static str,
    pub version: &'static str,
    pub executable: &'static str,
}

pub(crate) fn identity(id: ToolId) -> ExecutableIdentity {
    macro_rules! identity {
        ($module:ident, $distribution:expr) => {
            ExecutableIdentity {
                distribution_sha256: $distribution,
                sha256: $module::SHA256,
                version: $module::VERSION,
                executable: $module::EXECUTABLE,
            }
        };
    }
    match id {
        ToolId::Bun => identity!(bun, bun::DISTRIBUTION_SHA256),
        ToolId::Node => identity!(node, node::DISTRIBUTION_SHA256),
        ToolId::OfficeCli => identity!(officecli, None),
    }
}
