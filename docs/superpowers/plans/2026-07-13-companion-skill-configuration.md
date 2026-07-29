# Companion Skill Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the interrupted P0 implementation so each desktop companion can safely enable opt-in global Skills and disable auto-inject Skills, with changes applied to its existing conversation on the next message.

**Architecture:** Persist only per-companion intent in `CompanionProfileConfig`; derive a sorted effective Skill set in the companion service; synchronize verified managed entries into the companion's fixed workspace; store the effective set in `conversation.extra.skills` and recycle the cached agent only when that snapshot changes. Keep self-evolved companion specialties on their existing `companion_skill` path.

**Tech Stack:** Rust, Tokio, Serde, Axum service layer, React, TypeScript, Arco Design, `bun:test`, Cargo tests.

## Global Constraints

- Existing companion profiles without `skills` must deserialize to an empty configuration.
- Empty configuration must preserve all auto-inject Skills.
- Missing opt-in Skill names stay in the profile but do not enter the active conversation snapshot.
- Workspace reconciliation must never delete a same-named user entry that cannot be proven to be managed.
- Public conversation PATCH must continue rejecting direct `extra.skills` edits.
- A Skill assignment must not expand tool, file, browser, dangerous-operation, or remote-channel permissions.
- Existing self-evolved Skill review, edit, gift, and teaching behavior must remain unchanged.

---

### Task 1: Normalize profile intent and effective Skill names

**Files:**
- Modify: `crates/backend/nomifun-companion/src/profile.rs`
- Modify: `crates/backend/nomifun-companion/src/companion.rs`
- Test: inline tests in both files

**Interfaces:**
- Consumes: `CompanionSkillConfig { enabled, disabled_auto }` and auto-inject names.
- Produces: `normalized_effective_skill_names(auto_names: impl IntoIterator<Item = String>, config: &CompanionSkillConfig) -> Vec<String>`.

- [ ] **Step 1: Write the failing normalization test**

```rust
#[test]
fn effective_skill_names_trim_deduplicate_and_exclude_auto() {
    let config = CompanionSkillConfig {
        enabled: vec![" mermaid ".into(), "mermaid".into(), " ".into()],
        disabled_auto: vec![" cron ".into()],
    };
    assert_eq!(
        normalized_effective_skill_names(vec!["cron".into(), "todo".into()], &config),
        vec!["mermaid", "todo"]
    );
}
```

- [ ] **Step 2: Run the test and confirm RED**

Run: `cargo test -p nomifun-companion effective_skill_names_trim_deduplicate_and_exclude_auto --target-dir target/codex-companion-skill`

Expected: FAIL because `normalized_effective_skill_names` does not exist.

- [ ] **Step 3: Implement the pure normalized merge**

```rust
pub(crate) fn normalized_effective_skill_names(
    auto_names: impl IntoIterator<Item = String>,
    config: &CompanionSkillConfig,
) -> Vec<String> {
    let disabled: HashSet<String> = config
        .disabled_auto
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    auto_names
        .into_iter()
        .chain(config.enabled.iter().map(|name| name.trim().to_owned()))
        .filter(|name| !name.is_empty() && !disabled.contains(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
```

Call this helper from `CompanionThreads::effective_skill_names`; preserve the existing warning-and-continue behavior when auto Skill discovery fails.

- [ ] **Step 4: Run focused profile and effective-set tests**

Run: `cargo test -p nomifun-companion profile::tests --target-dir target/codex-companion-skill && cargo test -p nomifun-companion effective_skill_names_trim_deduplicate_and_exclude_auto --target-dir target/codex-companion-skill`

Expected: PASS.

### Task 2: Make workspace Skill ownership verifiable

**Files:**
- Create: `crates/backend/nomifun-companion/src/managed_skills.rs`
- Modify: `crates/backend/nomifun-companion/src/lib.rs`
- Modify: `crates/backend/nomifun-companion/src/companion.rs`
- Test: inline tests in `managed_skills.rs`

**Interfaces:**
- Consumes: resolved `ResolvedAgentSkill { name, source_path }` values and `{workspace}/.nomi/skills`.
- Produces: `ManagedSkillManifest`, `remove_stale_managed_entries(...)`, and `record_managed_entry(...)` helpers used only by companion reconciliation.

- [ ] **Step 1: Write failing ownership tests**

```rust
#[test]
fn stale_manifest_does_not_delete_user_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let skills = temp.path().join(".nomi/skills");
    let target = skills.join("mermaid");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("user.txt"), "mine").unwrap();
    let manifest = manifest_with_copy("mermaid", "C:/source/mermaid", "managed-token");

    remove_stale_managed_entries(&skills, &manifest, &HashSet::new());

    assert!(target.join("user.txt").exists());
}

#[test]
fn matching_copy_marker_allows_stale_managed_copy_removal() {
    let temp = tempfile::tempdir().unwrap();
    let skills = temp.path().join(".nomi/skills");
    let target = skills.join("mermaid");
    std::fs::create_dir_all(&target).unwrap();
    write_copy_marker(&target, "managed-token").unwrap();
    let manifest = manifest_with_copy("mermaid", "C:/source/mermaid", "managed-token");

    remove_stale_managed_entries(&skills, &manifest, &HashSet::new());

    assert!(!target.exists());
}
```

- [ ] **Step 2: Run both tests and confirm RED**

Run: `cargo test -p nomifun-companion managed_skills::tests --target-dir target/codex-companion-skill`

Expected: FAIL because the module and ownership helpers do not exist.

- [ ] **Step 3: Implement a versioned manifest**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedSkillRecord {
    source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    copy_token: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ManagedSkillManifest {
    #[serde(default)]
    managed: BTreeMap<String, ManagedSkillRecord>,
}
```

For links and junctions, compare `read_link(target)` (canonicalized when possible) with `record.source`. For physical copy fallback, create `.nomifun-managed-skill.json` containing a generated token and require the token to match before deletion. Invalid names containing `/`, `\\`, or `..` are ignored. When ownership cannot be verified, keep the target and omit it from the next manifest.

- [ ] **Step 4: Route `sync_workspace_skills` through the helper**

Resolve desired Skills first, remove only verified stale records, call `link_workspace_skills` only for missing targets, then inspect each created target and write the new manifest atomically. Never claim a pre-existing target as managed.

- [ ] **Step 5: Run managed workspace tests**

Run: `cargo test -p nomifun-companion managed_skills::tests --target-dir target/codex-companion-skill`

Expected: PASS, including user replacement preservation and managed-copy removal.

### Task 3: Harden conversation snapshot replacement

**Files:**
- Modify: `crates/backend/nomifun-conversation/src/service.rs`
- Modify: `crates/backend/nomifun-conversation/src/service_test.rs`

**Interfaces:**
- Consumes: `replace_skill_snapshot(conversation_id: &str, skills: &[String])`.
- Produces: normalized `extra.skills`, `Ok(true)` only on change, and one task recycle on change.

- [ ] **Step 1: Write failing snapshot tests**

```rust
fn make_service_with_concrete_task_manager() -> (
    ConversationService,
    Arc<MockRepo>,
    Arc<MockTaskManager>,
) {
    let repo = Arc::new(MockRepo::new());
    let task_mgr = Arc::new(MockTaskManager::new());
    let svc = ConversationService::new(
        std::env::temp_dir(),
        Arc::new(MockBroadcaster::new()),
        Arc::new(FixedSkillResolver { names: vec![] }),
        task_mgr.clone(),
        repo.clone(),
        Arc::new(StubAgentMetadataRepo),
        Arc::new(StubAcpSessionRepo::default()),
    );
    (svc, repo, task_mgr)
}

#[tokio::test]
async fn replace_skill_snapshot_updates_and_recycles_only_on_change() {
    let (svc, repo, task_mgr) = make_service_with_concrete_task_manager();
    let conv = svc.create("u", make_create_req()).await.unwrap();
    assert!(svc.replace_skill_snapshot(&conv.id.to_string(), &["pdf".into(), "pdf".into()]).await.unwrap());
    assert_eq!(task_mgr.kill_count(), 1);
    assert!(!svc.replace_skill_snapshot(&conv.id.to_string(), &["pdf".into()]).await.unwrap());
    assert_eq!(task_mgr.kill_count(), 1);
    let row = repo.get(conv.id).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(&row.extra).unwrap()["skills"], json!(["pdf"]));
}

#[tokio::test]
async fn replace_skill_snapshot_repairs_non_object_extra() {
    let (svc, repo, _) = make_service_with_concrete_task_manager();
    let conv = svc.create("u", make_create_req()).await.unwrap();
    repo.update(
        conv.id,
        &ConversationRowUpdate { extra: Some("[]".into()), ..Default::default() },
    ).await.unwrap();
    assert!(svc.replace_skill_snapshot(&conv.id.to_string(), &["pdf".into()]).await.unwrap());
    assert_eq!(serde_json::from_str::<Value>(&repo.get(conv.id).await.unwrap().unwrap().extra).unwrap()["skills"], json!(["pdf"]));
}
```

- [ ] **Step 2: Run both tests and confirm RED**

Run: `cargo test -p nomifun-conversation replace_skill_snapshot --target-dir target/codex-companion-skill`

Expected: the malformed-extra test fails or panics because array JSON cannot accept a string key.

- [ ] **Step 3: Normalize the `extra` container before assignment**

```rust
let mut extra = serde_json::from_str::<serde_json::Value>(&existing.extra)
    .ok()
    .filter(serde_json::Value::is_object)
    .unwrap_or_else(|| serde_json::json!({}));
```

Keep sorting, deduplication, no-op detection, repository update, and one best-effort `task_manager.kill` call.

- [ ] **Step 4: Run snapshot and public-PATCH regression tests**

Run: `cargo test -p nomifun-conversation replace_skill_snapshot --target-dir target/codex-companion-skill && cargo test -p nomifun-conversation update_rejects_extra_skills --target-dir target/codex-companion-skill`

Expected: PASS.

### Task 4: Isolate and test frontend assignment rules

**Files:**
- Create: `ui/src/renderer/pages/nomi/tabs/companionSkillConfig.ts`
- Create: `ui/src/renderer/pages/nomi/tabs/companionSkillConfig.test.ts`
- Create: `ui/src/renderer/pages/nomi/tabs/SkillsTab.configuration.test.ts`
- Modify: `ui/src/renderer/pages/nomi/tabs/SkillsTab.tsx`
- Modify: `ui/src/common/adapter/ipcBridge.ts`
- Modify: `ui/src/renderer/pages/nomi/useNomi.ts`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/nomi.json`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/nomi.json`
- Modify: `ui/src/renderer/services/i18n/i18n-keys.d.ts`

**Interfaces:**
- Consumes: `ICompanionSkillConfig`, auto Skill name set, selected name and checked state.
- Produces: `toggleCompanionSkill(config, autoNames, name, checked): ICompanionSkillConfig`.

- [ ] **Step 1: Write failing pure-function tests**

```ts
test('disabling an auto skill records only disabled_auto', () => {
  expect(toggleCompanionSkill({ enabled: [], disabled_auto: [] }, new Set(['cron']), 'cron', false))
    .toEqual({ enabled: [], disabled_auto: ['cron'] });
});

test('enabling an opt-in skill records only enabled', () => {
  expect(toggleCompanionSkill({ enabled: [], disabled_auto: [] }, new Set(), 'mermaid', true))
    .toEqual({ enabled: ['mermaid'], disabled_auto: [] });
});
```

- [ ] **Step 2: Run the test and confirm RED**

Run: `cd ui && bun test src/renderer/pages/nomi/tabs/companionSkillConfig.test.ts`

Expected: FAIL because the helper module does not exist.

- [ ] **Step 3: Implement and use the pure helper**

```ts
export function toggleCompanionSkill(
  config: ICompanionSkillConfig,
  autoNames: ReadonlySet<string>,
  name: string,
  checked: boolean
): ICompanionSkillConfig {
  const enabled = new Set(config.enabled);
  const disabledAuto = new Set(config.disabled_auto);
  if (autoNames.has(name)) checked ? disabledAuto.delete(name) : disabledAuto.add(name);
  else checked ? enabled.add(name) : enabled.delete(name);
  return { enabled: [...enabled].sort(), disabled_auto: [...disabledAuto].sort() };
}
```

Use the helper in `SkillsTab`; keep save-in-progress disabling, missing Skill rendering, search, and the separate “Configured capabilities” / “Companion specialties” sections.

- [ ] **Step 4: Add the structure regression test**

Read `SkillsTab.tsx` as UTF-8 and assert it contains the two i18n section keys, both catalog IPC calls, the missing-state key, and `toggleCompanionSkill`.

- [ ] **Step 5: Run frontend tests and checks**

Run: `cd ui && bun test src/renderer/pages/nomi/tabs/companionSkillConfig.test.ts src/renderer/pages/nomi/tabs/SkillsTab.configuration.test.ts && bun run typecheck`

Expected: PASS.

### Task 5: Verify the integrated P0 slice

**Files:**
- No planned file modifications; any unexpected failure stops completion and is diagnosed before editing.

- [ ] **Step 1: Format and check generated contracts**

Run: `cargo fmt --check && bun run check:i18n && git diff --check`

Expected: PASS with no formatting or i18n drift.

- [ ] **Step 2: Run focused backend suites in the isolated Cargo target**

Run: `cargo test -p nomifun-companion --lib --target-dir target/codex-companion-skill && cargo test -p nomifun-conversation --lib --target-dir target/codex-companion-skill`

Expected: PASS with zero failed tests.

- [ ] **Step 3: Run frontend regression and production build**

Run: `cd ui && bun test src/renderer/pages/nomi/tabs && bun run typecheck && bun run build`

Expected: PASS; Vite exits 0.

- [ ] **Step 4: Review the final diff against the design**

Confirm profile persistence, effective-set derivation, fixed-workspace sync, immutable public snapshot semantics, agent recycle, missing Skill UI, and unchanged companion-specialty actions are all present. Confirm no unrelated user changes were staged.
