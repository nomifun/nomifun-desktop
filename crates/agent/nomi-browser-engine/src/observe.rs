use std::collections::HashMap;
use std::io::{self, Write};

use serde::Serialize;

use crate::engine::{ElementEntry, Observation};

/// Hard payload ceiling for one task's in-flight/retained observation.
///
/// This is deliberately scoped to a single observation generation, not to the
/// browser process or all concurrent tasks.  Independent tasks may each use
/// their own allowance; one hostile page may not multiply a 64 MiB CDP
/// by-value response across hundreds of frames or keep an oversized snapshot in
/// the facade/Gateway caches.
pub const MAX_OBSERVATION_RETAINED_BYTES: usize = 4 * 1024 * 1024;

/// Payload-only capacity error.  It intentionally contains no page-controlled
/// string, URL, accessible name, or CDP response fragment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("observation byte capacity exceeded (limit={limit}, attempted={attempted})")]
pub struct ObservationCapacityError {
    pub limit: usize,
    pub attempted: usize,
}

impl ObservationCapacityError {
    pub fn new(attempted: usize) -> Self {
        Self {
            limit: MAX_OBSERVATION_RETAINED_BYTES,
            attempted,
        }
    }
}

fn checked_add(total: &mut usize, additional: usize) -> Result<(), ObservationCapacityError> {
    let attempted = total.saturating_add(additional);
    if attempted > MAX_OBSERVATION_RETAINED_BYTES {
        return Err(ObservationCapacityError::new(attempted));
    }
    *total = attempted;
    Ok(())
}

/// Check a materialized output before it is retained or copied into another
/// layer.  Callers use bytes (`str::len`), never Unicode scalar counts.
pub fn ensure_observation_bytes(bytes: usize) -> Result<(), ObservationCapacityError> {
    if bytes > MAX_OBSERVATION_RETAINED_BYTES {
        Err(ObservationCapacityError::new(bytes))
    } else {
        Ok(())
    }
}

/// Count JSON bytes through a sink which never stores the serialized payload.
/// This lets the platform reject escaping expansion *before* constructing its
/// `serde_json::Value` copy.
pub fn serialized_json_bytes_bounded<T: Serialize + ?Sized>(
    value: &T,
) -> Result<usize, ObservationCapacityError> {
    struct BoundedCounter {
        bytes: usize,
        attempted: usize,
    }

    impl Write for BoundedCounter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let attempted = self.bytes.saturating_add(buf.len());
            self.attempted = attempted;
            if attempted > MAX_OBSERVATION_RETAINED_BYTES {
                return Err(io::Error::other("observation byte capacity exceeded"));
            }
            self.bytes = attempted;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut counter = BoundedCounter {
        bytes: 0,
        attempted: 0,
    };
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(counter.bytes),
        Err(_) => Err(ObservationCapacityError::new(
            counter
                .attempted
                .max(MAX_OBSERVATION_RETAINED_BYTES.saturating_add(1)),
        )),
    }
}

/// 单帧 incrementalAriaSnapshot 的反序列化形态（call_injected 返回 JSON）。
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FrameSnapshot {
    pub full: String,
    #[serde(default)]
    pub incremental: Option<String>,
    #[serde(default, rename = "iframeRefs")]
    pub iframe_refs: Vec<String>,
    #[serde(default, rename = "iframeDepths")]
    pub iframe_depths: HashMap<String, u32>,
}

impl FrameSnapshot {
    /// Conservative heap bytes retained by the Rust representation.  Capacity,
    /// rather than length, catches allocator over-allocation too.
    pub fn retained_bytes(&self) -> Result<usize, ObservationCapacityError> {
        let mut total = 0usize;
        checked_add(&mut total, self.full.capacity())?;
        if let Some(incremental) = &self.incremental {
            checked_add(&mut total, incremental.capacity())?;
        }
        checked_add(
            &mut total,
            self.iframe_refs
                .capacity()
                .saturating_mul(std::mem::size_of::<String>()),
        )?;
        for reference in &self.iframe_refs {
            checked_add(&mut total, reference.capacity())?;
        }
        checked_add(
            &mut total,
            self.iframe_depths.capacity().saturating_mul(
                std::mem::size_of::<String>()
                    + std::mem::size_of::<u32>()
                    + std::mem::size_of::<usize>(),
            ),
        )?;
        for reference in self.iframe_depths.keys() {
            checked_add(&mut total, reference.capacity())?;
        }
        Ok(total)
    }
}

impl Observation {
    /// Conservative heap bytes kept alive by one cached Observation.
    pub fn retained_bytes(&self) -> Result<usize, ObservationCapacityError> {
        let mut total = 0usize;
        checked_add(&mut total, self.yaml.capacity())?;
        if let Some(url) = &self.url {
            checked_add(&mut total, url.capacity())?;
        }
        checked_add(
            &mut total,
            self.entries
                .capacity()
                .saturating_mul(std::mem::size_of::<ElementEntry>()),
        )?;
        for entry in &self.entries {
            checked_add(&mut total, entry.r#ref.capacity())?;
            checked_add(&mut total, entry.role.capacity())?;
            checked_add(&mut total, entry.name.capacity())?;
        }
        checked_add(
            &mut total,
            self.boxes.capacity().saturating_mul(
                std::mem::size_of::<String>()
                    + std::mem::size_of::<crate::engine::CssRect>()
                    + std::mem::size_of::<usize>(),
            ),
        )?;
        for reference in self.boxes.keys() {
            checked_add(&mut total, reference.capacity())?;
        }
        Ok(total)
    }

    pub fn validate_retained_bytes(&self) -> Result<(), ObservationCapacityError> {
        self.retained_bytes().map(|_| ())
    }
}

/// 把父 YAML 与各子帧 YAML（已含各自 refPrefix）缝合成一棵树。
/// children: (iframe_ref, child_yaml)。按 iframe_depths 给的 depth 缩进子内容。
/// Every append is checked before allocation; callers cannot accidentally
/// reintroduce an unbounded iframe stitch outside the CDP backend.
pub fn stitch(
    parent: &FrameSnapshot,
    children: &[(String, String)],
) -> Result<String, ObservationCapacityError> {
    if children.is_empty() {
        ensure_observation_bytes(parent.full.len())?;
        return Ok(parent.full.clone());
    }
    let child_map: HashMap<&str, &str> =
        children.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut out = String::new();
    let mut first = true;
    for line in parent.full.lines() {
        let matched = extract_iframe_ref(line).and_then(|iref| {
            let child = *child_map.get(iref)?;
            let depth = *parent.iframe_depths.get(iref)?;
            Some((child, depth))
        });
        if !first {
            append_bounded(&mut out, "\n")?;
        }
        first = false;
        if let Some((child, depth)) = matched {
            let head = line.trim_end();
            append_bounded(&mut out, head)?;
            if !head.ends_with(':') {
                append_bounded(&mut out, ":")?;
            }
            let indent = usize::try_from(depth)
                .ok()
                .and_then(|value| value.checked_add(1))
                .and_then(|value| value.checked_mul(2))
                .ok_or_else(|| {
                    ObservationCapacityError::new(
                        MAX_OBSERVATION_RETAINED_BYTES.saturating_add(1),
                    )
                })?;
            ensure_observation_bytes(indent)?;
            for cl in child.lines() {
                append_bounded(&mut out, "\n")?;
                append_spaces_bounded(&mut out, indent)?;
                append_bounded(&mut out, cl)?;
            }
            continue;
        }
        append_bounded(&mut out, line)?;
    }
    Ok(out)
}

fn append_bounded(out: &mut String, value: &str) -> Result<(), ObservationCapacityError> {
    ensure_observation_bytes(out.len().saturating_add(value.len()))?;
    out.push_str(value);
    Ok(())
}

fn append_spaces_bounded(
    out: &mut String,
    count: usize,
) -> Result<(), ObservationCapacityError> {
    ensure_observation_bytes(out.len().saturating_add(count))?;
    const SPACES: &str = "                                                                ";
    let mut remaining = count;
    while remaining != 0 {
        let chunk = remaining.min(SPACES.len());
        out.push_str(&SPACES[..chunk]);
        remaining -= chunk;
    }
    Ok(())
}

/// 从形如 `  - iframe [ref=f0e5]` 的行抽出 ref。
fn extract_iframe_ref(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if !t.starts_with("- iframe") && !t.contains("iframe ") {
        return None;
    }
    let start = line.find("[ref=")? + 5;
    let end = line[start..].find(']')? + start;
    Some(&line[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn stitch_inlines_child_frame_with_indent() {
        let parent = FrameSnapshot {
            full: "- generic:\n  - button \"Open\" [ref=f0e1]\n  - iframe [ref=f0e5]".into(),
            incremental: None,
            iframe_refs: vec!["f0e5".into()],
            iframe_depths: HashMap::from([("f0e5".to_string(), 1u32)]),
        };
        let out = stitch(&parent, &[("f0e5".to_string(), "- link \"Inner\" [ref=f1e1]".to_string())])
            .expect("small stitch fits");
        let expected = "- generic:\n  - button \"Open\" [ref=f0e1]\n  - iframe [ref=f0e5]:\n    - link \"Inner\" [ref=f1e1]";
        assert_eq!(out, expected);
    }
    #[test]
    fn stitch_no_iframe_returns_parent_full() {
        let p = FrameSnapshot { full: "- button \"X\" [ref=f0e1]".into(), incremental: None, iframe_refs: vec![], iframe_depths: HashMap::new() };
        assert_eq!(stitch(&p, &[]).expect("small stitch fits"), p.full);
    }

    #[test]
    fn hostile_single_accessible_name_is_rejected() {
        let snapshot = FrameSnapshot {
            full: format!(
                "- button \"{}\" [ref=f0e1]",
                "x".repeat(MAX_OBSERVATION_RETAINED_BYTES)
            ),
            incremental: None,
            iframe_refs: vec![],
            iframe_depths: HashMap::new(),
        };
        assert!(snapshot.retained_bytes().is_err());
    }

    #[test]
    fn observation_counts_yaml_and_entries_as_separate_retained_allocations() {
        let half = MAX_OBSERVATION_RETAINED_BYTES / 2;
        let observation = Observation {
            generation: crate::engine::SnapshotGen(1),
            yaml: "y".repeat(half),
            entries: vec![ElementEntry {
                r#ref: "f0e1".into(),
                role: "button".into(),
                name: "n".repeat(half),
                frame_seq: 0,
            }],
            url: None,
            truncated: false,
            current_page_is_post: false,
            boxes: HashMap::new(),
        };
        assert!(observation.validate_retained_bytes().is_err());
    }

    #[test]
    fn json_escaping_expansion_is_rejected_without_materializing_output() {
        // NUL occupies one byte in the retained Value but six JSON bytes
        // (`\\u0000`). The counting writer must enforce returned bytes.
        let value = serde_json::json!({
            "yaml": "\0".repeat(MAX_OBSERVATION_RETAINED_BYTES / 5)
        });
        assert!(serialized_json_bytes_bounded(&value).is_err());
    }
}
