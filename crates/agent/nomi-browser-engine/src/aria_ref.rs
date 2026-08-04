//! `f<seq>e<n>` ref 表 + 代际翻新（纯逻辑，无 CDP / 无 IO）。
//!
//! P1 的 observe 把页面表示成 aria-snapshot YAML；每个可操作元素用跨帧稳定的
//! ref `f<seq>e<n>`（frame 前缀 + frame-local 序号）锚定。本模块是 Rust 侧权威的
//! ref→记录 表：observe 每次重拍都新建一张表（`new_generation`），代际单调递增
//! （`wrapping_add(1)` 跳 0 哨兵，镜像 `nomi-a11y` windows actor 的写法）。
//! 旧代际的表与新代际相互隔离——新表为空，act 引用旧 ref 解析不到即视为 stale。

use std::collections::HashMap;

use crate::engine::SnapshotGen;

/// A single observe generation may expose at most this many live refs.
///
/// `find_elements` adds refs to the same generation as the latest observe.  A
/// per-call result cap is not a retention bound: repeated calls would otherwise
/// grow both the injected JavaScript map and this Rust table forever.  Keep the
/// generation finite and require a fresh observe when it is exhausted.
pub const MAX_REFS_PER_GENERATION: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("ref capacity exhausted for the current observe generation (limit={limit})")]
pub struct RefTableCapacityError {
    pub limit: usize,
}

/// 帧前缀：`seq=3` -> `"f3"`。frame-local 序号在注入侧拼接为 `f<seq>e<n>`。
pub fn frame_prefix(seq: u32) -> String {
    format!("f{seq}")
}

/// ref 表的一行：把一个 LLM-facing ref 解析回它的来源帧 + 角色/名字。
#[derive(Clone, Debug)]
pub struct RefRecord {
    /// 该元素所属帧的 CDP session id（act 路由 CDP 命令时用）。
    pub session_id: String,
    /// 该元素所属帧的 frame id。
    pub frame_id: String,
    /// 注入侧渲染的完整 ref（含 prefix）；P2 的 `aria-ref=` 反查用。
    pub full_ref: String,
    pub role: String,
    pub name: String,
}

/// 某一代际下的权威 ref 表：`"f3e7"`（LLM-facing ref）→ [`RefRecord`]。
/// observe 每次重拍都 `new_generation` 出一张全新（空）表；旧表与新表代际不同，
/// 故旧 ref 在新表中解析不到——这正是 stale 检测的依据。
#[derive(Clone, Debug)]
pub struct RefTable {
    generation: SnapshotGen,
    /// key = LLM-facing ref（如 `"f3e7"`）。
    map: HashMap<String, RefRecord>,
}

impl RefTable {
    /// 新代际的空表：generation 单调递增，`wrapping_add(1)` 跳 0 哨兵
    /// （`SnapshotGen(0)` 保留为初始哨兵，镜像 `nomi-a11y` windows actor）。
    pub fn new_generation(prev: Option<&RefTable>) -> Self {
        let next = match prev {
            None => SnapshotGen(1),
            Some(p) => {
                let n = p.generation.0.wrapping_add(1);
                SnapshotGen(if n == 0 { 1 } else { n })
            }
        };
        Self {
            generation: next,
            map: HashMap::new(),
        }
    }

    /// 登记一条 ref→记录。`llm_ref` 是 `f<seq>e<n>` 形式的 LLM-facing 句柄。
    pub fn insert(&mut self, llm_ref: &str, rec: RefRecord) {
        // Observe's existing parser predates fallible insertion.  Preserve its
        // API while making the storage invariant structural: entries beyond the
        // hard generation bound are never retained.  Incremental callers that
        // must report saturation use `try_insert`/`try_insert_batch` below.
        let _ = self.try_insert(llm_ref, rec);
    }

    /// Fallibly register one ref without evicting any existing ref.
    pub fn try_insert(
        &mut self,
        llm_ref: &str,
        rec: RefRecord,
    ) -> Result<(), RefTableCapacityError> {
        if !self.map.contains_key(llm_ref) && self.map.len() >= MAX_REFS_PER_GENERATION {
            return Err(RefTableCapacityError {
                limit: MAX_REFS_PER_GENERATION,
            });
        }
        self.map.insert(llm_ref.to_string(), rec);
        Ok(())
    }

    /// Atomically add a batch.  Capacity is checked before any mutation, so a
    /// failed `find_elements` registration cannot leave a half-published batch.
    pub fn try_insert_batch(
        &mut self,
        records: Vec<(String, RefRecord)>,
    ) -> Result<(), RefTableCapacityError> {
        let additional = records
            .iter()
            .map(|(key, _)| key.as_str())
            .filter(|key| !self.map.contains_key(*key))
            .collect::<std::collections::HashSet<_>>()
            .len();
        if self.map.len().saturating_add(additional) > MAX_REFS_PER_GENERATION {
            return Err(RefTableCapacityError {
                limit: MAX_REFS_PER_GENERATION,
            });
        }
        for (key, record) in records {
            self.map.insert(key, record);
        }
        Ok(())
    }

    /// Remove refs that the injected map proved are no longer connected.
    pub fn remove_refs<'a>(&mut self, refs: impl IntoIterator<Item = &'a str>) {
        for llm_ref in refs {
            self.map.remove(llm_ref);
        }
    }

    /// Invalidate every ref while retaining the generation number for error
    /// reporting.  The next observe replaces the table with a new generation.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 把一个 LLM-facing ref 解析回它的记录；未登记（含旧代际遗留 ref）返回 None。
    pub fn resolve(&self, llm_ref: &str) -> Option<&RefRecord> {
        self.map.get(llm_ref)
    }

    /// 本表里属于某帧（`frame_id`）的全部 ref（C3 find_elements 取主帧某 ref 推前缀用）。
    /// 顺序不保证（HashMap 迭代序）；调用方只需取「任一」该帧 ref。
    pub fn refs_for_frame<'a>(&'a self, frame_id: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.map
            .iter()
            .filter(move |(_, rec)| rec.frame_id == frame_id)
            .map(|(k, _)| k.as_str())
    }

    /// 本表所属代际。
    pub fn generation(&self) -> SnapshotGen {
        self.generation
    }

    /// 仅测试：强制设代际，用于覆盖 wrapping 跳 0 边界。
    #[cfg(test)]
    pub fn force_generation(&mut self, g: SnapshotGen) {
        self.generation = g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::SnapshotGen;

    #[test]
    fn frame_prefix_renders() {
        assert_eq!(frame_prefix(0), "f0");
        assert_eq!(frame_prefix(3), "f3");
    }

    #[test]
    fn generation_skips_zero_sentinel_and_wraps() {
        let g1 = RefTable::new_generation(None);
        assert_eq!(g1.generation(), SnapshotGen(1));
        let g2 = RefTable::new_generation(Some(&g1));
        assert_eq!(g2.generation(), SnapshotGen(2));
        let mut gmax = RefTable::new_generation(None);
        gmax.force_generation(SnapshotGen(u64::MAX));
        let after = RefTable::new_generation(Some(&gmax));
        assert_eq!(after.generation(), SnapshotGen(1)); // wrapping 跳 0 哨兵
    }

    #[test]
    fn insert_resolve_and_generation_isolation() {
        let mut t = RefTable::new_generation(None);
        t.insert(
            "f3e7",
            RefRecord {
                session_id: "S".into(),
                frame_id: "F".into(),
                full_ref: "f3e7".into(),
                role: "button".into(),
                name: "OK".into(),
            },
        );
        assert_eq!(t.resolve("f3e7").unwrap().role, "button");
        assert!(t.resolve("f9e1").is_none());
        let t2 = RefTable::new_generation(Some(&t));
        assert!(t2.resolve("f3e7").is_none()); // 新代际不含旧 ref
    }

    #[test]
    fn refs_for_frame_filters_by_frame_id() {
        // C3 find_elements 取主帧某 ref 推前缀：refs_for_frame 只返该帧的 ref。
        let mut t = RefTable::new_generation(None);
        let mk = |frame: &str, full: &str| RefRecord {
            session_id: "S".into(),
            frame_id: frame.into(),
            full_ref: full.into(),
            role: "button".into(),
            name: "X".into(),
        };
        t.insert("f0e1", mk("MAIN", "f0e1"));
        t.insert("f0e2", mk("MAIN", "f0e2"));
        t.insert("f1e1", mk("CHILD", "f1e1"));
        let main_refs: std::collections::HashSet<&str> = t.refs_for_frame("MAIN").collect();
        assert_eq!(main_refs.len(), 2);
        assert!(main_refs.contains("f0e1") && main_refs.contains("f0e2"));
        let child_refs: Vec<&str> = t.refs_for_frame("CHILD").collect();
        assert_eq!(child_refs, vec!["f1e1"]);
        // 无此帧 → 空迭代器。
        assert_eq!(t.refs_for_frame("NOPE").count(), 0);
    }

    #[test]
    fn generation_capacity_rejects_without_evicting_or_partial_batch() {
        let mut table = RefTable::new_generation(None);
        let record = |n: usize| RefRecord {
            session_id: "S".into(),
            frame_id: "F".into(),
            full_ref: format!("f0e{n}"),
            role: "button".into(),
            name: n.to_string(),
        };
        for n in 0..MAX_REFS_PER_GENERATION {
            table
                .try_insert(&format!("f0e{n}"), record(n))
                .expect("within bound");
        }
        assert_eq!(table.len(), MAX_REFS_PER_GENERATION);
        assert_eq!(
            table.try_insert("f0e-over", record(MAX_REFS_PER_GENERATION)),
            Err(RefTableCapacityError {
                limit: MAX_REFS_PER_GENERATION
            })
        );
        assert!(table.resolve("f0e0").is_some(), "capacity must never evict");
        assert!(table.resolve("f0e-over").is_none());
        table.insert("f0e-over", record(MAX_REFS_PER_GENERATION));
        assert!(
            table.resolve("f0e-over").is_none(),
            "legacy infallible insertion must still enforce the structural bound"
        );

        table.remove_refs(["f0e0"]);
        table
            .try_insert_batch(vec![
                ("f0e-new".into(), record(MAX_REFS_PER_GENERATION + 1)),
            ])
            .expect("disconnected-ref removal releases one slot");
        assert!(table.resolve("f0e-new").is_some());

        let before = table.len();
        let batch = vec![
            ("f0e-a".into(), record(MAX_REFS_PER_GENERATION + 2)),
            ("f0e-b".into(), record(MAX_REFS_PER_GENERATION + 3)),
        ];
        assert!(table.try_insert_batch(batch).is_err());
        assert_eq!(table.len(), before, "failed batch must be atomic");
        assert!(table.resolve("f0e-a").is_none() && table.resolve("f0e-b").is_none());
    }
}
