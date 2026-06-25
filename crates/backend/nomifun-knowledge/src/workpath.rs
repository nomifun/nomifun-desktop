//! Workpath keys — the canonical identifier a `workpath`-kind knowledge
//! binding is stored under (session-list unification spec §7).
//!
//! Knowledge mounts are physically per-workspace, so the binding belongs to
//! the workspace path ("workpath"), not to the individual session. Every
//! session whose workspace normalizes to the same key shares one binding
//! row `('workpath', key)`; backend-managed temporary workspaces all map to
//! the [`DEFAULT_WORKPATH_KEY`] sentinel.
//!
//! KEEP IN SYNC with the TypeScript twin
//! `ui/src/renderer/pages/conversation/SessionList/utils/workpathKey.ts` —
//! both sides must derive byte-identical keys or reads and writes land on
//! different rows. The normalization rules (and the test set) are the same
//! on both sides: trim → empty ⇒ sentinel → backslashes to forward slashes
//! → a bare `/` root is kept → trailing slashes stripped. No case folding.

use std::path::Path;

/// Binding `target_kind` for workpath-level knowledge bindings.
pub const WORKPATH_BINDING_KIND: &str = "workpath";

/// Sentinel `target_id` for the default workpath (backend-managed temporary
/// workspaces). Same constant as the TS side's `DEFAULT_WORKPATH_KEY`.
pub const DEFAULT_WORKPATH_KEY: &str = "__default__";

/// Normalize a workspace path into its binding key. Mirrors the TS
/// `workpathKey()` exactly (see module docs): trim → empty ⇒
/// [`DEFAULT_WORKPATH_KEY`] → `\` ⇒ `/` → root `/` kept → trailing slashes
/// stripped.
pub fn workpath_key(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return DEFAULT_WORKPATH_KEY.to_owned();
    }
    let slashed = trimmed.replace('\\', "/");
    if slashed == "/" {
        return slashed;
    }
    slashed.trim_end_matches('/').to_owned()
}

/// The workpath key a SESSION's workspace resolves to: a backend-managed
/// (temporary) workspace — one under `managed_root` — is the default
/// workpath ([`DEFAULT_WORKPATH_KEY`]); anything else keys by its
/// normalized path.
///
/// The "is temporary" test matches the derivations the session DTOs already
/// expose: conversations' `is_temporary_workspace`
/// (`nomifun-conversation/src/convert.rs`, `workspace` under the backend
/// data dir) and terminals' `is_default_workpath`
/// (`nomifun-terminal/src/types.rs`, `cwd` under the backend work dir) —
/// both guard against an empty root, since every path "starts with" an
/// empty prefix.
pub fn session_workpath_key(workspace: &Path, managed_root: &Path) -> String {
    if workspace.as_os_str().is_empty() {
        return DEFAULT_WORKPATH_KEY.to_owned();
    }
    if !managed_root.as_os_str().is_empty() && workspace.starts_with(managed_root) {
        return DEFAULT_WORKPATH_KEY.to_owned();
    }
    workpath_key(&workspace.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same case set as the TS twin's `workpathKey.test.ts` — keep both in
    /// sync when normalization rules change.
    #[test]
    fn workpath_key_matches_ts_normalization() {
        // 去尾斜杠
        assert_eq!(workpath_key("/Users/a/proj/"), "/Users/a/proj");
        // 保留根路径
        assert_eq!(workpath_key("/"), "/");
        // 空白输入归 default
        assert_eq!(workpath_key(""), DEFAULT_WORKPATH_KEY);
        assert_eq!(workpath_key("   "), DEFAULT_WORKPATH_KEY);
        // 不做大小写折叠
        assert_eq!(workpath_key("/Users/A"), "/Users/A");
        // Windows 反斜杠归一为正斜杠并去尾
        assert_eq!(workpath_key("C:\\work\\proj\\"), "C:/work/proj");
        // 已规范化的 key 再归一化是幂等的（服务端二次归一化依赖这一点）。
        assert_eq!(workpath_key(DEFAULT_WORKPATH_KEY), DEFAULT_WORKPATH_KEY);
        assert_eq!(workpath_key("/Users/a/proj"), "/Users/a/proj");
    }

    #[test]
    fn session_workpath_key_maps_temp_workspaces_to_default() {
        let root = Path::new("/data");
        // 临时工作区（位于 data_dir/work_dir 下）→ 哨兵。
        assert_eq!(
            session_workpath_key(Path::new("/data/conversations/gemini-temp-1"), root),
            DEFAULT_WORKPATH_KEY
        );
        // workspace == root 也算默认（starts_with 覆盖相等）。
        assert_eq!(session_workpath_key(root, root), DEFAULT_WORKPATH_KEY);
        // 空 workspace → 哨兵。
        assert_eq!(session_workpath_key(Path::new(""), root), DEFAULT_WORKPATH_KEY);
        // 用户自选目录 → 归一化 key（含去尾斜杠）。
        assert_eq!(session_workpath_key(Path::new("/Users/a/proj/"), root), "/Users/a/proj");
        // 空 root 不能把所有路径都吸成默认。
        assert_eq!(session_workpath_key(Path::new("/Users/a/proj"), Path::new("")), "/Users/a/proj");
    }
}
