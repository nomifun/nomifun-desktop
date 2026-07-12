# Managed Provider CRUD Test Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair the authenticated provider CRUD end-to-end test so it reflects the always-provisioned `nomifun-free-model` provider while preserving coverage of user-provider CRUD.

**Architecture:** Keep production behavior and shared application setup unchanged. Update only the integration-test assertions to validate exact provider counts and stable provider-ID membership without depending on repository ordering.

**Tech Stack:** Rust, Tokio, Axum, serde_json, Cargo test

## Global Constraints

- Modify only `provider_full_crud_with_auth` in `crates/backend/nomifun-app/tests/system_provider_e2e.rs`.
- Do not change production provider or managed-model behavior.
- Preserve existing create, update, authentication, API-key, and delete assertions.
- On Windows, add Git for Windows `sh` to the test process `PATH` and provide a temporary `C:\tmp` for the full suite, then remove it.

---

### Task 1: Align Provider CRUD Assertions with the Managed Provider

**Files:**
- Modify: `crates/backend/nomifun-app/tests/system_provider_e2e.rs:18-76`
- Test: `crates/backend/nomifun-app/tests/system_provider_e2e.rs`

**Interfaces:**
- Consumes: `GET /api/providers` JSON responses whose `data` field is an array of provider objects with stable string `id` fields.
- Produces: An end-to-end test that validates the reserved provider remains visible throughout user-provider CRUD.

- [ ] **Step 1: Run the existing test to verify the red baseline**

Run:

```powershell
$env:PATH = 'C:\Program Files\Git\bin;' + $env:PATH
cargo test -p nomifun-app --test system_provider_e2e provider_full_crud_with_auth -- --exact --nocapture
```

Expected: FAIL at the initial list assertion because `data` contains provider ID `nomifun-free-model` instead of being empty.

- [ ] **Step 2: Update the initial-list assertion**

Replace the empty-array assertion with:

```rust
let providers = json["data"].as_array().unwrap();
assert_eq!(providers.len(), 1);
assert_eq!(providers[0]["id"], "nomifun-free-model");
```

- [ ] **Step 3: Update the post-create list assertion**

Replace the single-count assertion with:

```rust
let providers = json["data"].as_array().unwrap();
assert_eq!(providers.len(), 2);
assert!(providers.iter().any(|provider| provider["id"] == "nomifun-free-model"));
assert!(providers.iter().any(|provider| provider["id"].as_str() == Some(id.as_str())));
```

- [ ] **Step 4: Update the post-delete assertion**

Replace the empty-array assertion with:

```rust
let providers = json["data"].as_array().unwrap();
assert_eq!(providers.len(), 1);
assert_eq!(providers[0]["id"], "nomifun-free-model");
```

- [ ] **Step 5: Run targeted verification**

Run:

```powershell
$env:PATH = 'C:\Program Files\Git\bin;' + $env:PATH
cargo test -p nomifun-app --test system_provider_e2e provider_full_crud_with_auth -- --exact
cargo test -p nomifun-app --test system_provider_e2e
cargo fmt --all -- --check
```

Expected: The single test and all 10 `system_provider_e2e` tests pass; formatting exits with code 0.

- [ ] **Step 6: Commit the test repair**

```powershell
git add crates/backend/nomifun-app/tests/system_provider_e2e.rs
git commit -m "test(app): account for managed provider in CRUD flow"
```

### Task 2: Verify and Push Main

**Files:**
- Verify only: entire Rust workspace

**Interfaces:**
- Consumes: committed `main` containing the feature merge, lockfile synchronization, design, and repaired test.
- Produces: remote `origin/main` at the verified local `main` commit.

- [ ] **Step 1: Run the full Rust test suite with Windows prerequisites**

Create `C:\tmp` only if absent, prepend `C:\Program Files\Git\bin` to `PATH`, run `cargo test --quiet`, and remove `C:\tmp` afterward if this run created it.

Expected: Exit code 0 with no failed tests. Repository warnings and explicitly ignored hardware/browser tests are permitted.

- [ ] **Step 2: Confirm repository state**

Run:

```powershell
git fetch --prune origin
git status --short --branch
git rev-list --left-right --count main...origin/main
```

Expected: `main` is not behind `origin/main`; the only untracked path is the pre-existing `build-task9-red/` build cache.

- [ ] **Step 3: Push main and verify remote alignment**

```powershell
git push origin main
git fetch origin main
git rev-list --left-right --count main...origin/main
```

Expected: Push succeeds and the final divergence count is `0 0`.
