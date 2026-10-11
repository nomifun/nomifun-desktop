//! Publication metadata belongs to each approved component artifact.
//! Small upstream notices are embedded once for standalone hosts and also packaged.
#[derive(Clone, Copy)]
pub(crate) struct PublicationFile<'a> {
    pub filename: &'a str,
    pub contents: &'a [u8],
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/component_notices.rs"));
}

pub(crate) fn files(id: crate::ToolId) -> &'static [PublicationFile<'static>] {
    match id {
        crate::ToolId::Bun => generated::BUN_FILES,
        crate::ToolId::Node => generated::NODE_FILES,
        crate::ToolId::OfficeCli => generated::OFFICECLI_FILES,
    }
}
