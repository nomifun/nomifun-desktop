# Turn Process Terminal Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the conversation turn header reflect the final completed or canceled outcome while retaining intermediate failures in the expandable process trace.

**Architecture:** Keep detailed process-item classification unchanged and resolve the aggregate state in `turnDisclosureModel.ts`, where turn closure and final assistant presence are known. Restrict the header to processed, canceled, processing, and waiting labels while preserving duration formatting for every state.

**Tech Stack:** React, TypeScript, Bun test runner, i18next, Vite.

## Global Constraints

- Every closed, non-canceled turn displays `Processed {{duration}}` / `已处理 {{duration}}`, even when execution ended with an error and produced no final text.
- A terminally canceled turn displays `Canceled {{duration}}` / `已取消 {{duration}}`.
- Intermediate failures remain visible in expanded details but cannot override later completion.
- Do not commit any changes.
- The shared implementation must work identically in macOS, Linux, Windows, and Web UI builds.

---

### Task 1: Lock terminal outcome precedence with regression tests

**Files:**

- Modify: `ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts`

**Interfaces:**

- Consumes: `buildTurnDisclosureItems(items, { tailClosed })`
- Produces: regression coverage for completed, canceled, and failed-detail normalization.

- [ ] **Step 1: Change the existing failed-then-final test to require `completed`**

Create a closed turn containing a failed process item and a final assistant item, then assert `disclosure.state === 'completed'` and the failed item remains `failed` in `processItemStates`.

- [ ] **Step 2: Add canceled terminal tests**

Create a closed turn whose latest terminal process item is canceled and has no final assistant response. Also cover an earlier failed item followed by cancellation. Assert the disclosure state is `canceled`, `running` is false, and the start/end timestamps preserve its execution duration.

- [ ] **Step 3: Add the closed failure normalization guard**

Create a closed turn containing a failed process item and no final assistant response. Assert its header state is `completed` while `processItemStates` retains `failed`.

- [ ] **Step 4: Run the focused test and verify RED**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts`

Expected: the failed-then-final assertion fails because the current model returns `failed`.

### Task 2: Implement terminal outcome resolution

**Files:**

- Modify: `ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.ts`

**Interfaces:**

- Consumes: per-item `TurnDisclosureProcessState`, `isClosed`, and `hasFinalAssistant`.
- Produces: aggregate `TurnDisclosureProcessState` with completion and cancellation terminal precedence.

- [ ] **Step 1: Add a focused terminal-state resolver**

When a turn is closed, preserve `canceled` when the final process item is canceled and normalize every other aggregate state to `completed`, while retaining per-item states.

- [ ] **Step 2: Keep live-turn precedence unchanged**

Keep `waiting` for a current confirmation prompt and normalize every other live aggregate state to `running`, so an intermediate failure cannot prematurely close the header.

- [ ] **Step 3: Run focused model tests and verify GREEN**

Run: `bun test ui/src/renderer/pages/conversation/Messages/turnDisclosureModel.test.ts ui/src/renderer/pages/conversation/Messages/turnProcessState.test.ts`

Expected: all tests pass.

### Task 3: Constrain header labels and preserve duration

**Files:**

- Modify: `ui/src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure.tsx`
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/messages.json`
- Modify: `ui/src/renderer/services/i18n/locales/en-US/messages.json`
- Test: `ui/src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure.expansion.test.ts`

**Interfaces:**

- Consumes: aggregate disclosure state and formatted `duration`.
- Produces: header text that uses processed/canceled/processing/waiting semantics and always interpolates duration.

- [ ] **Step 1: Add a label-contract test**

Assert the component maps completed to `messages.turnProcessed`, canceled to `messages.turnCanceled`, and does not map a terminal state to success wording.

- [ ] **Step 2: Remove the failed header copy path**

Map defensive `failed` rendering to the neutral processed label only where the header receives legacy or malformed aggregate data. Keep detailed process item failure styles untouched.

- [ ] **Step 3: Preserve duration interpolation for canceled state**

Keep `{{duration}}` in both Chinese and English canceled strings.

- [ ] **Step 4: Run component tests**

Run: `bun test ui/src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure.expansion.test.ts ui/src/renderer/pages/conversation/Messages/turnProcessLayout.structure.test.ts`

Expected: all tests pass.

### Task 4: Cross-platform shared-code verification

**Files:**

- Verify only; no platform-specific source changes expected.

**Interfaces:**

- Consumes: shared `ui` package.
- Produces: one artifact path used by macOS, Linux, Windows, and Web UI packaging.

- [ ] **Step 1: Run all conversation-message tests**

Run: `bun test ui/src/renderer/pages/conversation/Messages`

Expected: all tests pass.

- [ ] **Step 2: Run UI type checking**

Use the exact UI type-check command declared by the repository package scripts and require exit code 0.

- [ ] **Step 3: Run the shared UI production build**

Use the exact UI build command declared by the repository package scripts and require exit code 0.

- [ ] **Step 4: Review platform boundaries and diff**

Confirm there are no `cfg(target_os)` or runtime platform branches between the changed model/component and the platform bundles. Run `git diff --check`, inspect `git diff`, and confirm `git status --short` shows only intended uncommitted changes.
