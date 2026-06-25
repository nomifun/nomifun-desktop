use std::collections::HashMap;

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

/// 把父 YAML 与各子帧 YAML（已含各自 refPrefix）缝合成一棵树。
/// children: (iframe_ref, child_yaml)。按 iframe_depths 给的 depth 缩进子内容。
pub fn stitch(parent: &FrameSnapshot, children: &[(String, String)]) -> String {
    if children.is_empty() {
        return parent.full.clone();
    }
    let child_map: HashMap<&str, &str> =
        children.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut out: Vec<String> = Vec::new();
    for line in parent.full.lines() {
        let matched = extract_iframe_ref(line).and_then(|iref| {
            let child = *child_map.get(iref.as_str())?;
            let depth = *parent.iframe_depths.get(&iref)?;
            Some((child, depth))
        });
        if let Some((child, depth)) = matched {
            let head = if line.trim_end().ends_with(':') {
                line.to_string()
            } else {
                format!("{}:", line.trim_end())
            };
            out.push(head);
            let indent = " ".repeat(((depth + 1) * 2) as usize);
            for cl in child.lines() {
                out.push(format!("{indent}{cl}"));
            }
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

/// 从形如 `  - iframe [ref=f0e5]` 的行抽出 ref。
fn extract_iframe_ref(line: &str) -> Option<String> {
    let t = line.trim_start();
    if !t.starts_with("- iframe") && !t.contains("iframe ") {
        return None;
    }
    let start = line.find("[ref=")? + 5;
    let end = line[start..].find(']')? + start;
    Some(line[start..end].to_string())
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
        let out = stitch(&parent, &[("f0e5".to_string(), "- link \"Inner\" [ref=f1e1]".to_string())]);
        let expected = "- generic:\n  - button \"Open\" [ref=f0e1]\n  - iframe [ref=f0e5]:\n    - link \"Inner\" [ref=f1e1]";
        assert_eq!(out, expected);
    }
    #[test]
    fn stitch_no_iframe_returns_parent_full() {
        let p = FrameSnapshot { full: "- button \"X\" [ref=f0e1]".into(), incremental: None, iframe_refs: vec![], iframe_depths: HashMap::new() };
        assert_eq!(stitch(&p, &[]), p.full);
    }
}
