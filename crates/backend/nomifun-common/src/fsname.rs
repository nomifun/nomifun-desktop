//! 由用户可见显示名（伙伴名等）派生的、文件系统安全的目录段。
//! 与纯 ASCII slug 不同：保留 CJK 与其它 Unicode 字母数字，只剔除文件系统
//! （尤其 Windows/NTFS）无法存进单个路径段的字符。

/// 段长上限（按字符数，非字节）：兼顾 CJK 可读与总路径长度。调用方会前缀一个
/// 稳定唯一 id（如 seq），故截断不会造成冲突，无需 hash 后缀。
const MAX_SEGMENT_CHARS: usize = 40;

/// Windows 在路径段中禁止的字符。
const WIN_ILLEGAL: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// 把显示名净化为「单个」文件系统安全目录段（不含 seq/id 前缀）。无任何安全字符
/// 残留时返回空串——调用方应在该情形退化为仅用稳定 id/seq。
///
/// 规则：Windows 非法字符 + ASCII 控制字符 + 内部空白 → `_`；连续 `_` 折叠为一；
/// 首尾的 `_`、`.`、空白裁掉（Windows 会静默吞掉结尾的点/空格）；按字符数上限
/// `MAX_SEGMENT_CHARS` 截断。CJK 与其它 Unicode 字母数字原样保留。
///
/// 注：Windows 保留名（CON/NUL/…）不在此处理——调用方的 seq 数字前缀
/// （如 `1_con`）天然使整段不再是保留名。
pub fn sanitize_dir_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_underscore = false;
    for ch in name.chars() {
        let mapped = if ch.is_control() || ch.is_whitespace() || WIN_ILLEGAL.contains(&ch) {
            '_'
        } else {
            ch
        };
        if mapped == '_' {
            if prev_underscore {
                continue; // 折叠连续下划线
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
        out.push(mapped);
    }
    let trim = |s: &str| {
        s.trim_matches(|c: char| c == '_' || c == '.' || c.is_whitespace())
            .to_string()
    };
    let trimmed = trim(&out);
    let capped: String = trimmed.chars().take(MAX_SEGMENT_CHARS).collect();
    trim(&capped)
}

#[cfg(test)]
mod tests {
    use super::sanitize_dir_segment as s;

    #[test]
    fn keeps_cjk() {
        assert_eq!(s("毛球"), "毛球");
    }
    #[test]
    fn replaces_windows_illegal_and_collapses() {
        assert_eq!(s("a/b\\c:d"), "a_b_c_d");
    }
    #[test]
    fn internal_whitespace_to_underscore() {
        assert_eq!(s("My  Bot"), "My_Bot");
    }
    #[test]
    fn trims_edge_dots_spaces_underscores() {
        assert_eq!(s("  ..毛球.. "), "毛球");
    }
    #[test]
    fn neutralizes_traversal() {
        assert_eq!(s("../etc"), "etc");
    }
    #[test]
    fn all_illegal_returns_empty() {
        assert_eq!(s("///"), "");
        assert_eq!(s("   "), "");
    }
    #[test]
    fn control_chars_to_underscore() {
        assert_eq!(s("a\u{0007}b"), "a_b");
    }
    #[test]
    fn caps_at_40_chars_by_char_count() {
        let name = "毛".repeat(50);
        assert_eq!(s(&name).chars().count(), 40);
    }
}
