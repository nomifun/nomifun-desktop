# Collaborator Model Stale-Reference Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove deleted provider/model references from every collaborator-model surface and persisted soft reference, while preventing stale or disabled models from entering new orchestration runs.

**Architecture:** Build one frontend reconciliation primitive that separates structurally retained references from currently executable references, then apply it to homepage and conversation state only after the provider catalog is ready. Clean historical conversation soft references from the provider-deletion coordinator, and validate every conversation-derived or explicit orchestration range against live backend provider summaries immediately before run creation.

**Tech Stack:** React 19, TypeScript 5.8, SWR 2, Bun test, Rust 2024, Tokio, SQLx/SQLite JSON1, Cargo tests.

## Global Constraints

- Deleted provider IDs and deleted model IDs must never be displayed, submitted, or executed.
- Disabled-but-existing provider/model references remain persisted, but are not displayed, submitted, or executed until re-enabled.
- Provider-catalog loading must never be interpreted as an empty catalog and must never erase saved selections.
- Existing valid collaborator order is preserved and duplicate references collapse to the first occurrence.
- Existing orchestration fleet snapshots are immutable audit history and are not rewritten.
- No new runtime dependencies or storage-format migration.

---

## File Structure

- Create `ui/src/renderer/pages/orchestrator/collaboratorModelRefs.ts`: pure model-ref identity, reconciliation, and equality helpers.
- Create `ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts`: frontend behavioral regression tests.
- Modify `ui/src/renderer/hooks/agent/useGoogleAuthModels.ts`: expose first-load readiness.
- Modify `ui/src/renderer/hooks/agent/useModelProviderList.ts`: expose configured providers separately from executable providers and a combined loading flag.
- Modify `ui/src/renderer/pages/orchestrator/useModelRange.ts`: expose configured/executable pair catalogs and loading state.
- Modify `ui/src/renderer/pages/guid/GuidPage.tsx`: reconcile homepage persisted collaborators before display and submission.
- Modify `ui/src/renderer/pages/guid/components/GuidCollaboratorSelector.tsx`: never pass unknown tokens to the select and disable interaction while catalog identity is unresolved.
- Modify `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`: reconcile per-conversation collaborators before display, rewrite, and main-model changes.
- Modify `ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts`: enforce active-only homepage range construction.
- Modify `ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts`: enforce active-only conversation range construction.
- Modify `crates/backend/nomifun-db/src/repository/conversation.rs`: add the soft-reference cleanup repository contract.
- Modify `crates/backend/nomifun-db/src/repository/sqlite_conversation.rs`: implement and test ordered JSON-array cleanup.
- Modify `crates/backend/nomifun-app/src/provider_deletion.rs`: clean conversation model ranges after provider deletion and test the coordinator behavior.
- Modify `crates/backend/nomifun-app/src/router/state.rs`: inject the conversation repository into the deletion coordinator.
- Modify `crates/backend/nomifun-gateway/src/caps_orchestrator.rs`: validate/filter model ranges against live provider summaries and test both persisted and explicit policies.

---

### Task 1: Frontend provider catalog and pure reconciliation primitive

**Files:**

- Create: `ui/src/renderer/pages/orchestrator/collaboratorModelRefs.ts`
- Create: `ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts`
- Modify: `ui/src/renderer/hooks/agent/useGoogleAuthModels.ts`
- Modify: `ui/src/renderer/hooks/agent/useModelProviderList.ts`
- Modify: `ui/src/renderer/pages/orchestrator/useModelRange.ts`

**Interfaces:**

- Produces: `reconcileModelRefs(refs, configuredPairs, availablePairs): ModelRefReconciliation`.
- Produces: `sameModelRefs(left, right): boolean`.
- Produces from `useModelRange`: `configuredPairs`, `allPairs`, and `isLoading`.

- [ ] **Step 1: Write failing pure-function tests**

Add tests with this behavioral shape:

```ts
const ref = (provider_id: string, model: string): TModelRef => ({ provider_id, model });

test('removes a deleted provider while preserving valid order', () => {
  const result = reconcileModelRefs(
    [ref('gone', 'g1'), ref('keep', 'k2'), ref('keep', 'k1')],
    [ref('keep', 'k1'), ref('keep', 'k2')],
    [ref('keep', 'k1'), ref('keep', 'k2')]
  );
  expect(result.retained).toEqual([ref('keep', 'k2'), ref('keep', 'k1')]);
  expect(result.active).toEqual(result.retained);
  expect(result.removed).toEqual([ref('gone', 'g1')]);
});

test('retains but deactivates a disabled model', () => {
  const result = reconcileModelRefs(
    [ref('p', 'disabled'), ref('p', 'enabled')],
    [ref('p', 'disabled'), ref('p', 'enabled')],
    [ref('p', 'enabled')]
  );
  expect(result.retained).toEqual([ref('p', 'disabled'), ref('p', 'enabled')]);
  expect(result.active).toEqual([ref('p', 'enabled')]);
  expect(result.removed).toEqual([]);
});
```

Also cover a deleted model within a surviving provider, duplicate collapse, and `sameModelRefs` order sensitivity.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
bun test ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts
```

Expected: FAIL because `collaboratorModelRefs.ts` and its exports do not exist.

- [ ] **Step 3: Implement the minimal pure reconciliation module**

Use the existing NUL-safe `encodePair` identity or an equivalent private identity function. The implementation must preserve first occurrence order:

```ts
export interface ModelRefReconciliation {
  retained: TModelRef[];
  active: TModelRef[];
  removed: TModelRef[];
}

export const reconcileModelRefs = (
  refs: TModelRef[],
  configuredPairs: TModelRef[],
  availablePairs: TModelRef[]
): ModelRefReconciliation => {
  const configured = new Set(configuredPairs.map(modelRefKey));
  const available = new Set(availablePairs.map(modelRefKey));
  const seen = new Set<string>();
  const retained: TModelRef[] = [];
  const active: TModelRef[] = [];
  const removed: TModelRef[] = [];
  for (const item of refs) {
    const key = modelRefKey(item);
    if (seen.has(key)) continue;
    seen.add(key);
    if (!configured.has(key)) {
      removed.push(item);
      continue;
    }
    retained.push(item);
    if (available.has(key)) active.push(item);
  }
  return { retained, active, removed };
};
```

- [ ] **Step 4: Verify the pure tests are GREEN**

Run the Step 2 command. Expected: all reconciliation tests pass.

- [ ] **Step 5: Expose catalog readiness and configured pairs**

In `useGoogleAuthModels`, return an `isLoading` flag derived from the Google config and auth-status SWR queries. In `useModelProviderList`, return:

```ts
interface ModelProviderListResult {
  providers: IProvider[];
  configuredProviders: IProvider[];
  isLoading: boolean;
  // existing helpers remain unchanged
}
```

`configuredProviders` contains the loaded backend rows before enabled/model-capability filtering. `providers` retains current executable filtering. In `useModelRange`, flatten `configuredProviders[].models` into `configuredPairs`, retain executable `allPairs`, and forward `isLoading`.

- [ ] **Step 6: Run focused tests and typecheck**

Run:

```bash
bun test ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts
bun --cwd ui run typecheck
```

Expected: tests pass and TypeScript exits 0.

- [ ] **Step 7: Commit Task 1**

```bash
git add ui/src/renderer/hooks/agent/useGoogleAuthModels.ts \
  ui/src/renderer/hooks/agent/useModelProviderList.ts \
  ui/src/renderer/pages/orchestrator/useModelRange.ts \
  ui/src/renderer/pages/orchestrator/collaboratorModelRefs.ts \
  ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts
git commit -m "fix(ui): reconcile collaborator model references"
```

---

### Task 2: Apply reconciliation to homepage, conversation, and selector

**Files:**

- Modify: `ui/src/renderer/pages/guid/GuidPage.tsx`
- Modify: `ui/src/renderer/pages/guid/components/GuidCollaboratorSelector.tsx`
- Modify: `ui/src/renderer/pages/conversation/components/ChatConversation.tsx`
- Modify: `ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts`
- Modify: `ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts`

**Interfaces:**

- Consumes: `reconcileModelRefs`, `sameModelRefs`, `configuredPairs`, `allPairs`, `isLoading` from Task 1.
- Produces: homepage and conversation ranges constructed only from `active` collaborator refs.

- [ ] **Step 1: Add failing integration structure assertions**

Require both parent surfaces to import `reconcileModelRefs`, derive an `activeCollaborators` value, use it in `GuidCollaboratorSelector`, and use it when constructing/persisting executable ranges. Require the selector to derive a set from `allPairs` and avoid pinning a main-model token absent from that set.

Representative assertions:

```ts
expect(pageSource.includes('const activeCollaborators = collaboratorReconciliation?.active ?? []')).toBe(true);
expect(pageSource.includes('value={activeCollaborators}')).toBe(true);
expect(pageSource.includes('...activeCollaborators.filter(')).toBe(true);

expect(chatSource.includes('const activeCollaborators = collaboratorReconciliation?.active ?? []')).toBe(true);
expect(chatSource.includes('void persistModelRange(mainModelRef, activeCollaborators)')).toBe(true);
```

- [ ] **Step 2: Run integration tests and verify RED**

```bash
bun test \
  ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts \
  ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts
```

Expected: FAIL on the new active-only assertions.

- [ ] **Step 3: Implement homepage reconciliation**

Call `useModelRange` in `GuidPage`, derive reconciliation only when `isLoading === false`, render `activeCollaborators`, and build `orchestratorModelRange` from active refs. Add an effect that calls the existing change handler with `retained` only when `removed.length > 0` and the arrays differ.

- [ ] **Step 4: Implement conversation reconciliation**

Derive the same reconciliation from the raw conversation collaborator state. Use active refs in the selector, main-model switch rewrite, and any newly submitted range. Add a ready-gated effect that updates local state and persists `retained` only for permanent removals.

- [ ] **Step 5: Harden the selector loading and pinned-main behavior**

Use `allPairs` and `isLoading` from `useModelRange`. During loading, pass an empty selection and disable the button. After loading, pin `mainModel` only when its encoded key exists in `allPairs`. Labels and hints must use the validated `value` prop.

- [ ] **Step 6: Verify Task 2 GREEN**

```bash
bun test \
  ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts \
  ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts \
  ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts
bun --cwd ui run typecheck
```

Expected: all focused tests pass and TypeScript exits 0.

- [ ] **Step 7: Commit Task 2**

```bash
git add ui/src/renderer/pages/guid/GuidPage.tsx \
  ui/src/renderer/pages/guid/components/GuidCollaboratorSelector.tsx \
  ui/src/renderer/pages/conversation/components/ChatConversation.tsx \
  ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts \
  ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts
git commit -m "fix(ui): purge stale collaborator selections"
```

---

### Task 3: Clean historical conversation ranges after provider deletion

**Files:**

- Modify: `crates/backend/nomifun-db/src/repository/conversation.rs`
- Modify: `crates/backend/nomifun-db/src/repository/sqlite_conversation.rs`
- Modify: `crates/backend/nomifun-app/src/provider_deletion.rs`
- Modify: `crates/backend/nomifun-app/src/router/state.rs`

**Interfaces:**

- Produces: `IConversationRepository::remove_provider_from_orchestrator_model_ranges(provider_id: &str) -> Result<u64, DbError>`.
- Consumes: that repository method from `AppProviderDeletionCoordinator::cleanup_soft_refs`.

- [ ] **Step 1: Add a failing SQLite repository test**

Create conversations containing:

```json
{
  "workspace": "/keep",
  "orchestrator_model_range": {
    "mode": "range",
    "models": [
      {"provider_id":"gone","model":"g1"},
      {"provider_id":"keep","model":"k1"},
      {"provider_id":"gone","model":"g2"}
    ]
  }
}
```

Assert the method reports one changed row, preserves `/keep`, and leaves only `keep/k1` in original order. Add unrelated and malformed-extra rows and assert they remain unchanged.

- [ ] **Step 2: Run the repository test and verify RED**

```bash
cargo test -p nomifun-db remove_provider_from_orchestrator_model_ranges -- --nocapture
```

Expected: FAIL because the repository method does not exist.

- [ ] **Step 3: Add the repository contract and SQLite implementation**

Give the trait a default `Ok(0)` implementation so lightweight mock repositories remain source compatible. The SQLite implementation must use JSON1 to rebuild only the target models array, retain array order, and guard with `json_valid`, `json_type(...)= 'array'`, and an `EXISTS` match before updating. Return `rows_affected()`.

- [ ] **Step 4: Verify repository GREEN**

Run the Step 2 command. Expected: the target cleanup tests pass.

- [ ] **Step 5: Add a failing deletion-coordinator integration test**

Inject a conversation repository into the test coordinator, create a conversation range referencing `prov_x` and `prov_keep`, call `cleanup_soft_refs("prov_x")`, and assert both the existing failover queue cleanup and conversation range cleanup occurred.

- [ ] **Step 6: Run the coordinator test and verify RED**

```bash
cargo test -p nomifun-app provider_deletion -- --nocapture
```

Expected: FAIL because `AppProviderDeletionCoordinator` does not yet own or call a conversation repository.

- [ ] **Step 7: Wire conversation cleanup into the coordinator**

Add:

```rust
pub conversation_repo: Arc<dyn IConversationRepository>,
```

Call `remove_provider_from_orchestrator_model_ranges(provider_id)` after failover cleanup. In `build_system_state`, reuse `services.conversation_repo.clone()` so the coordinator points at the same database.

- [ ] **Step 8: Verify Task 3 GREEN**

```bash
cargo test -p nomifun-db remove_provider_from_orchestrator_model_ranges -- --nocapture
cargo test -p nomifun-app provider_deletion -- --nocapture
```

Expected: all targeted database and coordinator tests pass.

- [ ] **Step 9: Commit Task 3**

```bash
git add crates/backend/nomifun-db/src/repository/conversation.rs \
  crates/backend/nomifun-db/src/repository/sqlite_conversation.rs \
  crates/backend/nomifun-app/src/provider_deletion.rs \
  crates/backend/nomifun-app/src/router/state.rs
git commit -m "fix(provider): clean conversation collaborator refs"
```

---

### Task 4: Validate orchestration ranges against live providers

**Files:**

- Modify: `crates/backend/nomifun-gateway/src/caps_orchestrator.rs`

**Interfaces:**

- Produces: pure `filter_persisted_model_range(range, summaries) -> Option<ModelRange>`.
- Produces: pure `validate_explicit_model_range(range, summaries) -> Result<ModelRange, Value>`.

- [ ] **Step 1: Add failing pure backend tests**

Cover these contracts:

```rust
#[test]
fn persisted_range_drops_deleted_and_disabled_pairs_preserving_order() {
    let providers = vec![
        summary("keep", true, &["k1", "k2"]),
        summary("disabled", false, &["d1"]),
    ];
    let range = ModelRange::Range {
        models: vec![
            ModelRef { provider_id: "gone".into(), model: "g1".into() },
            ModelRef { provider_id: "keep".into(), model: "k2".into() },
            ModelRef { provider_id: "disabled".into(), model: "d1".into() },
            ModelRef { provider_id: "keep".into(), model: "k1".into() },
        ],
    };
    assert_eq!(
        filter_persisted_model_range(range, &providers),
        Some(ModelRange::Range {
            models: vec![
                ModelRef { provider_id: "keep".into(), model: "k2".into() },
                ModelRef { provider_id: "keep".into(), model: "k1".into() },
            ],
        })
    );
}

#[test]
fn persisted_range_returns_none_when_no_pair_survives() {
    let range = ModelRange::Single {
        model: ModelRef { provider_id: "gone".into(), model: "g1".into() },
    };
    assert_eq!(filter_persisted_model_range(range, &[summary("keep", true, &["k1"])]), None);
}

#[test]
fn explicit_range_rejects_any_unavailable_pair() {
    let range = ModelRange::Range {
        models: vec![
            ModelRef { provider_id: "keep".into(), model: "k1".into() },
            ModelRef { provider_id: "gone".into(), model: "g1".into() },
        ],
    };
    let error = validate_explicit_model_range(range, &[summary("keep", true, &["k1"])]).unwrap_err();
    let message = error["error"].as_str().unwrap_or_default();
    assert!(message.contains("gone/g1"), "got: {message}");
}

#[test]
fn filtered_range_lead_is_first_surviving_pair() {
    let range = ModelRange::Range {
        models: vec![
            ModelRef { provider_id: "gone".into(), model: "g1".into() },
            ModelRef { provider_id: "keep".into(), model: "k2".into() },
        ],
    };
    let filtered = filter_persisted_model_range(range, &[summary("keep", true, &["k2"])]).unwrap();
    let ModelRange::Range { models } = filtered else { panic!("expected range") };
    assert_eq!(models.first().map(|item| item.provider_id.as_str()), Some("keep"));
    assert_eq!(models.first().map(|item| item.model.as_str()), Some("k2"));
}
```

- [ ] **Step 2: Run tests and verify RED**

```bash
cargo test -p nomifun-gateway caps_orchestrator -- --nocapture
```

Expected: compile/test failure because the validation helpers do not exist.

- [ ] **Step 3: Implement pure availability validation**

Build the available identity set only from `ProviderSummary { enabled: true, models }`. Persisted ranges filter unavailable refs and deduplicate in original order; `Single` returns `None` when invalid. Explicit `Single`/`Range` returns a JSON error naming every unavailable pair instead of silently filtering. `Auto` remains unchanged for later expansion.

- [ ] **Step 4: Apply validation to conversation-derived ranges**

Load provider summaries before accepting `extra.orchestrator_model_range`. Change `read_conversation_model_range` to filter the persisted range; when empty, validate the conversation main model; when that is also unavailable, return `None` so the existing Auto fallback expands live providers.

- [ ] **Step 5: Apply validation to explicit and flat-spawn paths**

Track whether `RunCreateParams.model_range` was explicitly supplied. Validate explicit ranges and return the clear error before creating a run. Update the conversation-native flat-spawn path to load summaries once and use the same persisted-range filter. Compute `lead_model` only after validation/filtering.

- [ ] **Step 6: Verify Task 4 GREEN**

```bash
cargo test -p nomifun-gateway caps_orchestrator -- --nocapture
cargo check -p nomifun-gateway
```

Expected: gateway tests pass and the crate checks successfully.

- [ ] **Step 7: Commit Task 4**

```bash
git add crates/backend/nomifun-gateway/src/caps_orchestrator.rs
git commit -m "fix(orchestrator): reject stale collaborator models"
```

---

### Task 5: Cross-layer verification and noise review

**Files:**

- Verify all files changed in Tasks 1–4.

**Interfaces:**

- Consumes all prior task outputs; produces no new behavior.

- [ ] **Step 1: Run the complete targeted regression set**

```bash
bun test \
  ui/src/renderer/pages/orchestrator/collaboratorModelRefs.test.ts \
  ui/src/renderer/pages/guid/GuidClusterControls.structure.test.ts \
  ui/src/renderer/pages/conversation/platforms/nomi/NomiSendBoxLayout.structure.test.ts
cargo test -p nomifun-db remove_provider_from_orchestrator_model_ranges -- --nocapture
cargo test -p nomifun-app provider_deletion -- --nocapture
cargo test -p nomifun-gateway caps_orchestrator -- --nocapture
```

Expected: every command exits 0 with zero failed tests.

- [ ] **Step 2: Run compile/static verification**

```bash
bun --cwd ui run typecheck
cargo check --workspace
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 3: Review the final diff against the specification**

Verify explicitly:

- deleted refs disappear from UI and persisted soft refs;
- disabled refs are retained but inactive;
- loading cannot write cleanup;
- backend filtering happens before lead calculation;
- explicit invalid ranges fail rather than silently changing intent;
- no unrelated formatting, dependency, or fleet-history changes are present.

- [ ] **Step 4: Commit any verification-only corrections**

If verification required a correction, rerun the affected RED/GREEN test and commit only the correction. If no correction was needed, do not create an empty commit.
