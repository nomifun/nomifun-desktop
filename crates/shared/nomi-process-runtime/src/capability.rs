use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxPolicy {
    UnrestrictedLocalOwner,
    MacSeatbelt { write_roots: Vec<PathBuf> },
    DenySpawn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityPolicy {
    pub cwd_roots: Vec<PathBuf>,
    pub sandbox: SandboxPolicy,
}

impl CapabilityPolicy {
    pub fn local_owner(root: PathBuf) -> Self {
        Self {
            cwd_roots: vec![root],
            sandbox: SandboxPolicy::UnrestrictedLocalOwner,
        }
    }
}
