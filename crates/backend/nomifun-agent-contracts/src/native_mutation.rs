//! Frozen facts captured from the native objects retained by an explicit file
//! mutation owner. Handles are process-local; none are persistent identity.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeMutationPlatform {
    Windows,
    Unix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeMutationSemantics {
    Entry,
    Follow,
}

/// Components are native comparison keys below an existing pinned parent.
/// Windows keys use OS Unicode upcase conservatively even in a case-sensitive
/// directory, so later case-mode changes cannot bypass an old footprint.
/// They are not lowercase input paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMutationNamespace {
    pub parent_id: String,
    pub components: Vec<Vec<u16>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMutationTarget {
    pub platform: NativeMutationPlatform,
    /// Native volume identity captured from the retained object/parent.
    /// A subtree conservatively covers this volume: enumerating descendants
    /// once cannot prove the absence of concurrent or later hardlink aliases.
    pub volume_id: String,
    /// Publication can create a new object identity after admission. Until
    /// settled, its aliases are not enumerable from the prepublication fact,
    /// so the frozen footprint conservatively covers this native volume.
    pub volume_wide: bool,
    /// A native filesystem namespace/volume and persistent file ID. Streams
    /// intentionally share the base object identity, conservatively fencing
    /// base-file and alternate-stream mutations against one another.
    pub object_id: Option<String>,
    pub namespace: NativeMutationNamespace,
    pub ancestor_ids: Vec<String>,
    pub subtree: bool,
    pub semantics: NativeMutationSemantics,
}

/// One atomic footprint for all targets of a single explicit file action.
/// This does not describe arbitrary process or shell side effects.
/// It coordinates cooperating admitted Windows file actions; concurrent
/// user/other-program file, link or staging edits are externally coordinated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMutationTargets {
    pub version: u32,
    pub targets: Vec<NativeMutationTarget>,
}
